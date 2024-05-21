use embedded_io::{Error, ErrorKind, ErrorType};
use embedded_io_async::{BufRead, Read};

use crate::TryBufRead;

pub(crate) struct ReadBuffer<'buf> {
    pub buffer: &'buf mut [u8],
    pub loaded: usize,
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

pub struct BufferingReader<'resp, 'buf, B>
where
    B: Read,
{
    pub(crate) buffer: ReadBuffer<'buf>,
    pub(crate) stream: &'resp mut B,
}

impl<'resp, 'buf, B> BufferingReader<'resp, 'buf, B>
where
    B: Read,
{
    pub fn new(buffer: &'buf mut [u8], loaded: usize, stream: &'resp mut B) -> Self {
        Self {
            buffer: ReadBuffer::new(buffer, loaded),
            stream,
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
        if !self.buffer.is_empty() {
            let amt = self.buffer.read(buf)?;
            return Ok(amt);
        }

        self.stream.read(buf).await.map_err(|e| e.kind())
    }
}

impl<C> BufRead for BufferingReader<'_, '_, C>
where
    C: TryBufRead,
{
    async fn fill_buf(&mut self) -> Result<&[u8], ErrorKind> {
        // We need to consume the loaded bytes before we read mode.
        if self.buffer.is_empty() {
            // The matches/if let dance is to fix lifetime of the borrowed inner connection.
            if self.stream.try_fill_buf().await.is_some() {
                if let Some(result) = self.stream.try_fill_buf().await {
                    return result.map_err(|e| e.kind());
                }
                unreachable!()
            }

            self.buffer.loaded = self.stream.read(self.buffer.buffer).await.map_err(|e| e.kind())?;
        }

        self.buffer.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        // It's possible that the user requested more bytes to be consumed than loaded. Especially
        // since it's also possible that nothing is loaded, after we consumed all and are using
        // embedded-tls's buffering.
        let unconsumed = self.buffer.consume(amt);

        if unconsumed > 0 {
            self.stream.try_consume(unconsumed);
        }
    }
}
