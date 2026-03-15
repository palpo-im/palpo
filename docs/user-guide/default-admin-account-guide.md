# Default Admin Account User Guide

## Table of Contents

1. [Initial Setup](#initial-setup)
2. [Logging In](#logging-in)
3. [Changing Your Password](#changing-your-password)
4. [Password Recovery](#password-recovery)
5. [Troubleshooting](#troubleshooting)
6. [Security Best Practices](#security-best-practices)

---

## Initial Setup

### First-Time Setup Wizard

When you access the Palpo Matrix Server Web Admin Interface for the first time, you will be presented with a setup wizard to configure your admin account.

### Step 1: Welcome Screen

The setup wizard will display a welcome message explaining that you need to set up your admin account.

**What to do**: Click "Next" to proceed to the password setup screen.

### Step 2: Set Initial Password

You will be prompted to enter a password for the admin account.

**Password Requirements**:
- **Minimum length**: 12 characters
- **Must include**:
  - At least one uppercase letter (A-Z)
  - At least one lowercase letter (a-z)
  - At least one digit (0-9)
  - At least one special character (!@#$%^&*)

**Example of a valid password**: `MySecurePass123!@#`

**What to do**:
1. Enter a password that meets all requirements
2. The password strength indicator will show your password strength
3. Click "Next" to proceed

### Step 3: Confirm Password

Re-enter your password to confirm it.

**What to do**:
1. Enter the same password again
2. Click "Next" to proceed

### Step 4: Completion

The setup wizard will confirm that your admin account has been successfully created.

**What to do**: Click "Login" to proceed to the login page.

---

## Logging In

### Accessing the Login Page

1. Open your web browser
2. Navigate to your Palpo Matrix Server Web Admin Interface URL (e.g., `http://localhost:8080`)
3. You will be redirected to the login page if you are not already logged in

### Entering Your Credentials

**Username**: Always use `admin` (this is the fixed username for the Web UI administrator)

**Password**: Enter the password you set during the initial setup

### Logging In

1. Enter `admin` in the username field
2. Enter your password in the password field
3. Click the "Login" button
4. If your credentials are correct, you will be logged in and redirected to the admin dashboard

### Troubleshooting Login Issues

**"Invalid username or password"**
- Verify that you entered the correct password
- Ensure the username is exactly `admin` (case-sensitive)
- Check that Caps Lock is not accidentally enabled

**"Admin account not yet configured"**
- The initial password has not been set up
- Run the setup wizard first

**"Too many login attempts"**
- You have exceeded the maximum number of login attempts
- Wait 15 minutes before trying again

---

## Changing Your Password

### When to Change Your Password

You should change your password:
- Regularly (e.g., every 90 days)
- If you suspect it has been compromised
- If you shared it with someone who should no longer have access

### How to Change Your Password

1. Log in to the admin dashboard
2. Click on your profile icon or settings menu (usually in the top-right corner)
3. Select "Change Password"
4. You will be taken to the password change page

### Password Change Form

**Current Password**: Enter your current password for verification

**New Password**: Enter your new password
- Must meet the same requirements as the initial password
- Must be different from your current password
- Password strength indicator will show your password strength

**Confirm New Password**: Re-enter your new password to confirm

### Completing the Password Change

1. Fill in all three fields
2. Click "Save" to change your password
3. You will see a confirmation message
4. You will be logged out automatically
5. Log in again with your new password

### Troubleshooting Password Change

**"Old password is incorrect"**
- Verify that you entered your current password correctly
- Check that Caps Lock is not accidentally enabled

**"Password does not meet policy requirements"**
- Ensure your new password has at least 12 characters
- Include uppercase, lowercase, digit, and special character
- See [Password Requirements](#password-requirements) for details

**"New password cannot be the same as old password"**
- Choose a different password
- You cannot reuse your current password

---

## Password Recovery

### Forgot Your Password?

If you forget your admin password, you can reset it by directly accessing the database. This requires:
- Access to the server where the database is running
- Database administration credentials
- Basic SQL knowledge

### Password Recovery Steps

#### Step 1: Connect to the Database

Connect to your PostgreSQL database using a database client (e.g., `psql`, pgAdmin, or DBeaver).

**Connection Details**:
- **Host**: Your database server hostname or IP address
- **Port**: 5432 (default PostgreSQL port)
- **Database**: Your admin database name (e.g., `palpo_admin`)
- **Username**: Your database admin username
- **Password**: Your database admin password

**Example using psql**:
```bash
psql -h localhost -U postgres -d palpo_admin
```

#### Step 2: Delete the Current Credentials

Run the following SQL command to delete the current admin credentials:

```sql
DELETE FROM webui_admin_credentials WHERE id = 1;
```

**What this does**: Removes the stored password hash, resetting the admin account to an uninitialized state.

#### Step 3: Verify Deletion

Verify that the credentials have been deleted:

```sql
SELECT * FROM webui_admin_credentials;
```

**Expected result**: No rows returned (empty result set)

#### Step 4: Restart the Application

Restart the Palpo Matrix Server Web Admin Interface application.

**How to restart**:
- If running as a service: `sudo systemctl restart palpo-admin-server`
- If running in Docker: `docker restart <container_name>`
- If running manually: Stop the process and start it again

#### Step 5: Run Setup Wizard Again

After restarting, the application will detect that the admin account is not initialized and will display the setup wizard.

1. Open your web browser and navigate to the admin interface
2. The setup wizard will appear automatically
3. Follow the [Initial Setup](#initial-setup) steps to create a new password

### Important Notes

⚠️ **Security Warning**: Password recovery requires direct database access. This is a security feature to prevent unauthorized password resets.

⚠️ **Backup First**: Before making any database changes, ensure you have a backup of your database.

⚠️ **Change Password After Recovery**: After recovering your password, change it to a new one as soon as possible.

---

## Troubleshooting

### Common Issues and Solutions

#### Issue: "Admin account not yet configured"

**Cause**: The initial password has not been set up.

**Solution**:
1. The setup wizard should appear automatically
2. Follow the [Initial Setup](#initial-setup) steps
3. If the wizard doesn't appear, try clearing your browser cache and refreshing the page

#### Issue: "Invalid username or password"

**Cause**: Incorrect credentials provided.

**Solution**:
1. Verify the username is exactly `admin`
2. Check that Caps Lock is not enabled
3. Ensure you entered the correct password
4. If you forgot your password, see [Password Recovery](#password-recovery)

#### Issue: "Too many login attempts"

**Cause**: You have exceeded the maximum number of login attempts (5 attempts in 15 minutes).

**Solution**:
1. Wait 15 minutes before trying again
2. Ensure you have the correct password before retrying
3. If you forgot your password, see [Password Recovery](#password-recovery)

#### Issue: "Password does not meet policy requirements"

**Cause**: Your password doesn't meet the required criteria.

**Solution**:
1. Ensure your password has at least 12 characters
2. Include at least one uppercase letter (A-Z)
3. Include at least one lowercase letter (a-z)
4. Include at least one digit (0-9)
5. Include at least one special character (!@#$%^&*)

**Example of a valid password**: `MySecurePass123!@#`

#### Issue: "Session expired"

**Cause**: Your session token has expired (default: 24 hours).

**Solution**:
1. Log in again with your credentials
2. You will receive a new session token

#### Issue: "Cannot connect to the server"

**Cause**: The admin server is not running or not accessible.

**Solution**:
1. Check that the admin server is running
2. Verify the server URL is correct
3. Check your network connection
4. Check firewall settings to ensure port 8080 is accessible
5. Check server logs for error messages

#### Issue: "Database connection error"

**Cause**: The admin server cannot connect to the database.

**Solution**:
1. Verify the database is running
2. Check database connection settings in the configuration file
3. Verify database credentials are correct
4. Check network connectivity to the database server
5. Check database logs for error messages

### Getting Help

If you encounter issues not covered in this guide:

1. Check the server logs for error messages
2. Review the [API Documentation](../api/default-admin-account-api.md) for technical details
3. Contact your system administrator

---

## Security Best Practices

### Password Security

1. **Use a Strong Password**
   - Follow all password requirements
   - Use a mix of character types
   - Avoid common words or patterns
   - Consider using a password manager

2. **Keep Your Password Secret**
   - Never share your password with anyone
   - Never write it down in plain text
   - Never include it in emails or messages

3. **Change Your Password Regularly**
   - Change your password every 90 days
   - Change it immediately if you suspect it's been compromised
   - Change it if someone who knew the password should no longer have access

4. **Use Unique Passwords**
   - Don't reuse passwords from other systems
   - Each system should have a unique password

### Session Security

1. **Log Out When Done**
   - Always log out when you finish using the admin interface
   - Especially important on shared computers

2. **Don't Leave Sessions Unattended**
   - Lock your computer when you step away
   - Sessions will automatically expire after 24 hours

3. **Use HTTPS**
   - Always access the admin interface over HTTPS in production
   - Never use HTTP for authentication

4. **Secure Your Browser**
   - Keep your browser and operating system updated
   - Use a reputable antivirus/anti-malware solution
   - Be cautious of phishing attempts

### Account Security

1. **Monitor Audit Logs**
   - Regularly review audit logs for suspicious activity
   - Look for unexpected login attempts or password changes

2. **Secure Database Access**
   - Restrict database access to authorized personnel only
   - Use strong database credentials
   - Change database passwords regularly

3. **Backup Your Configuration**
   - Regularly backup your admin database
   - Store backups in a secure location
   - Test backup restoration procedures

### Network Security

1. **Use a Firewall**
   - Restrict access to the admin interface to authorized networks
   - Use a firewall to block unauthorized access

2. **Use VPN**
   - Consider using a VPN for remote access
   - Ensures encrypted communication

3. **Monitor Network Traffic**
   - Monitor for suspicious network activity
   - Use intrusion detection systems if available

---

## Additional Resources

- [API Documentation](../api/default-admin-account-api.md) - Technical API reference
- [Design Document](.kiro/specs/default-admin-account/design.md) - Technical architecture details
- [Requirements Document](.kiro/specs/default-admin-account/requirements.md) - Feature requirements

---

## FAQ

### Q: Can I change the username from "admin"?

**A**: No, the username is fixed as "admin" for security reasons. This is the only Web UI administrator account.

### Q: How long does a session last?

**A**: Sessions expire after 24 hours by default. You will need to log in again after expiration.

### Q: Can I have multiple admin accounts?

**A**: No, there is only one admin account for the Web UI. This is by design to simplify security management.

### Q: What happens if I forget my password?

**A**: You can reset your password by deleting the credentials from the database and restarting the application. See [Password Recovery](#password-recovery) for detailed steps.

### Q: Is my password stored securely?

**A**: Yes, passwords are hashed using bcrypt/argon2 and never stored in plain text. Even administrators cannot see your password.

### Q: Can I export my audit logs?

**A**: Yes, audit logs are stored in the database and can be queried or exported using standard database tools.

### Q: What should I do if I suspect my password has been compromised?

**A**: Change your password immediately using the password change feature. Review audit logs for suspicious activity.

### Q: Can I disable the admin account?

**A**: No, the admin account cannot be disabled. It is required to manage the Web UI.

### Q: What is the password policy?

**A**: Passwords must be at least 12 characters and include uppercase, lowercase, digit, and special character. See [Password Requirements](#password-requirements) for details.

---

## Version Information

**Document Version**: 1.0

**Last Updated**: 2024-03-06

**Applicable To**: Palpo Matrix Server Web Admin Interface v1.0+

