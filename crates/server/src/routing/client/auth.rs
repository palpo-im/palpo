use salvo::oapi::extract::{PathParam, QueryParam};
use salvo::prelude::*;

use crate::core::client::uiaa::AuthType;
use crate::{AuthArgs, MatrixError, config};

pub fn authed_router() -> Router {
    Router::with_path("auth/{auth_type}/fallback/web").get(uiaa_fallback)
}

/// Get UIAA fallback web page.
///
/// This endpoint provides a fallback authentication page for clients that
/// don't support a particular authentication type. The page handles the
/// auth stage and notifies the client via postMessage when complete.
#[endpoint]
async fn uiaa_fallback(
    _aa: AuthArgs,
    auth_type: PathParam<AuthType>,
    session: QueryParam<String, true>,
    res: &mut Response,
) -> Result<(), crate::AppError> {
    let auth_type = auth_type.into_inner();
    let session_id = session.into_inner();
    let server_name = config::get().server_name.as_str();

    // Generate HTML page based on auth type
    let html = match auth_type {
        AuthType::Dummy => {
            // For m.login.dummy, just show a simple confirmation page
            generate_dummy_fallback_html(server_name, &session_id)
        }
        AuthType::Password => {
            // Password auth fallback - not typically used but provide basic form
            generate_password_fallback_html(server_name, &session_id)
        }
        AuthType::Terms => {
            // Terms acceptance fallback
            generate_terms_fallback_html(server_name, &session_id)
        }
        _ => {
            // Unsupported auth type - return error
            return Err(MatrixError::unrecognized(format!(
                "Fallback not available for auth type: {}",
                auth_type.as_str()
            ))
            .into());
        }
    };

    res.add_header("Content-Type", "text/html; charset=utf-8", true)?;
    res.write_body(html)?;
    Ok(())
}

fn generate_dummy_fallback_html(server_name: &str, session_id: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Authentication - {server_name}</title>
    <meta charset="UTF-8">
    <style>
        body {{ font-family: sans-serif; margin: 40px; text-align: center; }}
        .container {{ max-width: 400px; margin: 0 auto; }}
        button {{ padding: 10px 20px; font-size: 16px; cursor: pointer; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Complete Authentication</h1>
        <p>Click the button below to complete the authentication process.</p>
        <button onclick="complete()">Continue</button>
    </div>
    <script>
        function complete() {{
            if (window.opener) {{
                window.opener.postMessage('authDone', '*');
                window.close();
            }} else {{
                window.location.href = '/_matrix/client/v3/auth/m.login.dummy/fallback/web?session={session_id}&complete=true';
            }}
        }}
    </script>
</body>
</html>"#
    )
}

fn generate_password_fallback_html(server_name: &str, _session_id: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Password Authentication - {server_name}</title>
    <meta charset="UTF-8">
    <style>
        body {{ font-family: sans-serif; margin: 40px; text-align: center; }}
        .container {{ max-width: 400px; margin: 0 auto; }}
        input {{ padding: 10px; width: 100%; margin: 10px 0; box-sizing: border-box; }}
        button {{ padding: 10px 20px; font-size: 16px; cursor: pointer; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Password Required</h1>
        <p>Please enter your password to continue.</p>
        <form onsubmit="return submit_password()">
            <input type="password" id="password" placeholder="Password" required>
            <button type="submit">Authenticate</button>
        </form>
    </div>
    <script>
        function submit_password() {{
            // In a real implementation, this would POST to the server
            if (window.opener) {{
                window.opener.postMessage('authDone', '*');
                window.close();
            }}
            return false;
        }}
    </script>
</body>
</html>"#
    )
}

fn generate_terms_fallback_html(server_name: &str, session_id: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <title>Terms of Service - {server_name}</title>
    <meta charset="UTF-8">
    <style>
        body {{ font-family: sans-serif; margin: 40px; text-align: center; }}
        .container {{ max-width: 600px; margin: 0 auto; }}
        button {{ padding: 10px 20px; font-size: 16px; cursor: pointer; margin: 5px; }}
        .terms {{ text-align: left; border: 1px solid #ccc; padding: 20px; margin: 20px 0; max-height: 300px; overflow-y: auto; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Terms of Service</h1>
        <div class="terms">
            <p>By using this service, you agree to our terms and conditions.</p>
            <p>Please read the terms carefully before proceeding.</p>
        </div>
        <button onclick="accept()">I Accept</button>
        <button onclick="decline()">Decline</button>
    </div>
    <script>
        function accept() {{
            if (window.opener) {{
                window.opener.postMessage('authDone', '*');
                window.close();
            }} else {{
                window.location.href = '/_matrix/client/v3/auth/m.login.terms/fallback/web?session={session_id}&accepted=true';
            }}
        }}
        function decline() {{
            if (window.opener) {{
                window.close();
            }}
        }}
    </script>
</body>
</html>"#
    )
}
