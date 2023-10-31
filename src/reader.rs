use buffered_io::asynch::BufferedRead;
use embedded_io::{Error, ErrorKind, ErrorType};
use embedded_io_async::{BufRead, Read, Write};

use crate::client::HttpConnection;

pub struct BufferingReader<'resp, 'buf, B>
where
    B: Read,
{
    buffered: BufferedRead<'buf, &'resp mut B>,
}

impl<'resp, 'buf, B> BufferingReader<'resp, 'buf, B>
where
    B: Read,
{
    pub fn new(buffer: &'buf mut [u8], loaded: usize, stream: &'resp mut B) -> Self {
        Self {
            buffered: BufferedRead::new_with_data(stream, buffer, 0, loaded),
        }
    }
}

impl<C> ErrorType for BufferingReader<'_, '_, C>
where
    C: Read,
{
    type Error = ErrorKind;
}

impl<C> Read for BufferingReader<'_, '_, C>
where
    C: Read,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.buffered.read(buf).await.map_err(|e| e.kind())
    }
}

impl<C> BufRead for BufferingReader<'_, '_, HttpConnection<'_, C>>
where
    C: Read + Write,
{
    async fn fill_buf(&mut self) -> Result<&[u8], ErrorKind> {
        // The call to buffered.bypass() will only return Ok(...) if the buffer is empty.
        // This ensures that we completely drain the possibly pre-filled buffer before we try
        // to use the embedded-tls buffer directly.
        // The matches/if let dance is to fix lifetime of the borrowed inner connection.
        #[cfg(feature = "embedded-tls")]
        if matches!(self.buffered.bypass(), Ok(HttpConnection::Tls(_))) {
            if let HttpConnection::Tls(ref mut tls) = self.buffered.bypass().unwrap() {
                return tls.fill_buf().await.map_err(|e| e.kind());
            }
            unreachable!();
        }

        self.buffered.fill_buf().await
    }

    fn consume(&mut self, amt: usize) {
        // The call to buffered.bypass() will only return Ok(...) if the buffer is empty.
        #[cfg(feature = "embedded-tls")]
        if let Ok(HttpConnection::Tls(tls)) = self.buffered.bypass() {
            tls.consume(amt);
            return;
        }

        self.buffered.consume(amt);
    }
}
