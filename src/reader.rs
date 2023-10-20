use embedded_io::{ErrorKind, ErrorType};
use embedded_io_async::{BufRead, Read, Write};

#[cfg(feature = "embedded-tls")]
use embedded_io::Error;

use crate::client::HttpConnection;

struct ReadBuffer<'buf> {
    buffer: &'buf mut [u8],
    loaded: usize,
}

impl<'buf> ReadBuffer<'buf> {
    fn new(buffer: &'buf mut [u8], loaded: usize) -> Self {
        Self { buffer, loaded }
    }
}

impl ReadBuffer<'_> {
    fn is_empty(&self) -> bool {
        self.loaded == 0
    }

    fn read(&mut self, buf: &mut [u8]) -> Result<usize, ErrorKind> {
        let amt = self.loaded.min(buf.len());
        buf[..amt].copy_from_slice(&self.buffer[0..amt]);

        self.consume(amt);

        Ok(amt)
    }

    fn fill_buf(&mut self) -> Result<&[u8], ErrorKind> {
        Ok(&self.buffer[..self.loaded])
    }

    fn consume(&mut self, amt: usize) -> usize {
        let to_consume = amt.min(self.loaded);

        self.buffer.copy_within(to_consume..self.loaded, 0);
        self.loaded -= to_consume;

        amt - to_consume
    }
}

pub struct BufferingReader<'buf, B> {
    buffer: ReadBuffer<'buf>,
    stream: B,
}

impl<'buf, B> BufferingReader<'buf, B> {
    pub fn new(buffer: &'buf mut [u8], loaded: usize, stream: B) -> Self {
        Self {
            buffer: ReadBuffer::new(buffer, loaded),
            stream,
        }
    }
}

impl<C> ErrorType for BufferingReader<'_, &mut HttpConnection<'_, C>>
where
    C: Read + Write,
{
    type Error = ErrorKind;
}

impl<C> Read for BufferingReader<'_, &mut HttpConnection<'_, C>>
where
    C: Read + Write,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        if !self.buffer.is_empty() {
            let amt = self.buffer.read(buf)?;
            return Ok(amt);
        }

        self.stream.read(buf).await
    }
}

impl<C> BufRead for BufferingReader<'_, &mut HttpConnection<'_, C>>
where
    C: Read + Write,
{
    async fn fill_buf(&mut self) -> Result<&[u8], ErrorKind> {
        // We need to consume the loaded bytes before we read mode.
        if self.buffer.is_empty() {
            // embedded-tls has its own internal buffer, let's prefer that if we can
            #[cfg(feature = "embedded-tls")]
            if let HttpConnection::Tls(ref mut tls) = self.stream {
                return tls.fill_buf().await.map_err(|e| e.kind());
            }

            self.buffer.loaded = self.stream.read(&mut self.buffer.buffer).await?;
        }

        self.buffer.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        // It's possible that the user requested more bytes to be consumed than loaded. Especially
        // since it's also possible that nothing is loaded, after we consumed all and are using
        // embedded-tls's buffering.
        let unconsumed = self.buffer.consume(amt);

        if unconsumed > 0 {
            #[cfg(feature = "embedded-tls")]
            if let HttpConnection::Tls(tls) = &mut self.stream {
                tls.consume(unconsumed);
            }
        }
    }
}
