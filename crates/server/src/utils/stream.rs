use bytes::Bytes;
use futures_util::stream::{Stream, TryStream};
use futures_util::{StreamExt, stream};

use crate::AppResult;
use crate::core::MatrixError;

fn ensure_response_size(current: usize, additional: usize, max_size: usize) -> AppResult<()> {
    if additional > max_size.saturating_sub(current) {
        return Err(MatrixError::too_large(format!(
            "Remote response exceeds the configured {max_size} byte limit",
        ))
        .into());
    }
    Ok(())
}

/// Read an HTTP response into memory without allowing it to exceed `max_size`.
///
/// `Content-Length` is used only for an early rejection. The streamed byte
/// count remains authoritative so chunked and incorrectly sized responses are
/// bounded as well.
pub async fn read_response_limited(
    mut response: reqwest::Response,
    max_size: usize,
) -> AppResult<Bytes> {
    if response
        .content_length()
        .is_some_and(|length| length > max_size as u64)
    {
        return Err(MatrixError::too_large(format!(
            "Remote response exceeds the configured {max_size} byte limit",
        ))
        .into());
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure_response_size(body.len(), chunk.len(), max_size)?;
        body.extend_from_slice(&chunk);
    }
    Ok(body.into())
}

pub trait IterStream<I: IntoIterator + Send> {
    /// Convert an Iterator into a Stream
    fn stream(self) -> impl Stream<Item = <I as IntoIterator>::Item> + Send;

    /// Convert an Iterator into a TryStream
    fn try_stream(
        self,
    ) -> impl TryStream<Ok = <I as IntoIterator>::Item, Error = crate::AppError> + Send;
}

impl<I> IterStream<I> for I
where
    I: IntoIterator + Send,
    <I as IntoIterator>::IntoIter: Send,
{
    #[inline]
    fn stream(self) -> impl Stream<Item = <I as IntoIterator>::Item> + Send {
        stream::iter(self)
    }

    #[inline]
    fn try_stream(
        self,
    ) -> impl TryStream<Ok = <I as IntoIterator>::Item, Error = crate::AppError> + Send {
        self.stream().map(Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(body: Vec<u8>, content_length: Option<usize>) -> reqwest::Response {
        let mut builder = http::Response::builder();
        if let Some(content_length) = content_length {
            builder = builder.header(http::header::CONTENT_LENGTH, content_length);
        }
        builder.body(body).unwrap().into()
    }

    #[tokio::test]
    async fn response_reader_accepts_bytes_up_to_limit() {
        let body = read_response_limited(response(vec![1; 10], None), 10)
            .await
            .unwrap();
        assert_eq!(body.len(), 10);
    }

    #[tokio::test]
    async fn response_reader_rejects_oversized_content_length() {
        assert!(
            read_response_limited(response(vec![1; 11], Some(11)), 10)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn response_reader_rejects_oversized_stream_without_length() {
        assert!(
            read_response_limited(response(vec![1; 11], None), 10)
                .await
                .is_err()
        );
    }

    #[test]
    fn response_size_check_handles_overflow() {
        assert!(ensure_response_size(usize::MAX, 1, usize::MAX).is_err());
    }
}
