use embedded_io_async::{BufRead, Error as _, ErrorType, Read};

use crate::{
    reader::{BufferingReader, ReadBuffer},
    Error, TryBufRead,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ChunkState {
    NoChunk,
    NotEmpty(u32),
    Empty,
}

impl ChunkState {
    fn consume(&mut self, amt: usize) -> usize {
        if let ChunkState::NotEmpty(remaining) = self {
            let consumed = (amt as u32).min(*remaining);
            *remaining -= consumed;
            consumed as usize
        } else {
            0
        }
    }

    fn len(self) -> usize {
        if let ChunkState::NotEmpty(len) = self {
            len as usize
        } else {
            0
        }
    }
}

/// Chunked response body reader
pub struct ChunkedBodyReader<B> {
    pub raw_body: B,
    chunk_remaining: ChunkState,
}

impl<C> ChunkedBodyReader<C>
where
    C: Read,
{
    pub fn new(raw_body: C) -> Self {
        Self {
            raw_body,
            chunk_remaining: ChunkState::NoChunk,
        }
    }

    pub fn is_done(&self) -> bool {
        self.chunk_remaining == ChunkState::Empty
    }

    async fn read_next_chunk_length(&mut self) -> Result<(), Error> {
        let mut header_buf = [0; 8 + 2]; // 32 bit hex + \r + \n
        let mut total_read = 0;

        'read_size: loop {
            let mut byte = 0;
            self.raw_body
                .read_exact(core::slice::from_mut(&mut byte))
                .await
                .map_err(|e| Error::from(e).kind())?;

            if byte != b'\n' {
                header_buf[total_read] = byte;
                total_read += 1;

                if total_read == header_buf.len() {
                    return Err(Error::Codec);
                }
            } else {
                if total_read == 0 || header_buf[total_read - 1] != b'\r' {
                    return Err(Error::Codec);
                }
                break 'read_size;
            }
        }

        let hex_digits = total_read - 1;

        // Prepend hex with zeros
        let mut hex = [b'0'; 8];
        hex[8 - hex_digits..].copy_from_slice(&header_buf[..hex_digits]);

        let mut bytes = [0; 4];
        hex::decode_to_slice(hex, &mut bytes).map_err(|_| Error::Codec)?;

        let chunk_length = u32::from_be_bytes(bytes);

        debug!("Chunk length: {}", chunk_length);

        self.chunk_remaining = match chunk_length {
            0 => ChunkState::Empty,
            other => ChunkState::NotEmpty(other),
        };

        Ok(())
    }

    async fn read_chunk_end(&mut self) -> Result<(), Error> {
        // All chunks are terminated with a \r\n
        let mut newline_buf = [0; 2];
        self.raw_body.read_exact(&mut newline_buf).await?;

        if newline_buf != [b'\r', b'\n'] {
            return Err(Error::Codec);
        }
        Ok(())
    }

    /// Handles chunk boundary and returns the number of bytes in the current (or new) chunk.
    async fn handle_chunk_boundary(&mut self) -> Result<usize, Error> {
        match self.chunk_remaining {
            ChunkState::NoChunk => self.read_next_chunk_length().await?,

            ChunkState::NotEmpty(0) => {
                // The current chunk is currently empty, advance into a new chunk...
                self.read_chunk_end().await?;
                self.read_next_chunk_length().await?;
            }

            ChunkState::NotEmpty(_) => {}

            ChunkState::Empty => return Ok(0),
        }

        if self.chunk_remaining == ChunkState::Empty {
            // Read final chunk termination
            self.read_chunk_end().await?;
        }

        Ok(self.chunk_remaining.len())
    }
}

impl<'conn, 'buf, C> ChunkedBodyReader<BufferingReader<'conn, 'buf, C>>
where
    C: Read + TryBufRead,
{
    pub(crate) async fn read_to_end(self) -> Result<&'buf mut [u8], Error> {
        let buffer = self.raw_body.buffer.buffer;

        // We reconstruct the reader to change the 'buf lifetime.
        let mut reader = ChunkedBodyReader {
            raw_body: BufferingReader {
                buffer: ReadBuffer {
                    buffer: &mut buffer[..],
                    loaded: self.raw_body.buffer.loaded,
                },
                stream: self.raw_body.stream,
            },
            chunk_remaining: self.chunk_remaining,
        };

        let mut len = 0;
        while !reader.raw_body.buffer.buffer.is_empty() {
            // Read some
            let read = reader.fill_buf().await?.len();
            len += read;

            // Make sure we don't erase the newly read data
            let was_loaded = reader.raw_body.buffer.loaded;
            let fake_loaded = read.min(was_loaded);
            reader.raw_body.buffer.loaded = fake_loaded;

            // Consume the returned buffer
            reader.consume(read);

            if reader.is_done() {
                // If we're done, we don't care about the rest of the housekeeping.
                break;
            }

            // How many bytes were actually consumed from the preloaded buffer?
            let consumed_from_buffer = fake_loaded - reader.raw_body.buffer.loaded;

            // ... move the buffer by that many bytes to avoid overwriting in the next iteration.
            reader.raw_body.buffer.loaded = was_loaded - consumed_from_buffer;
            reader.raw_body.buffer.buffer = &mut reader.raw_body.buffer.buffer[consumed_from_buffer..];
        }

        if !reader.is_done() {
            return Err(Error::BufferTooSmall);
        }

        Ok(&mut buffer[..len])
    }
}

impl<C> ErrorType for ChunkedBodyReader<C> {
    type Error = Error;
}

impl<C> Read for ChunkedBodyReader<C>
where
    C: Read,
{
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let remaining = self.handle_chunk_boundary().await?;
        let max_len = buf.len().min(remaining);

        let len = self
            .raw_body
            .read(&mut buf[..max_len])
            .await
            .map_err(|e| Error::Network(e.kind()))?;

        self.chunk_remaining.consume(len);

        Ok(len)
    }
}

impl<C> BufRead for ChunkedBodyReader<C>
where
    C: BufRead + Read,
{
    async fn fill_buf(&mut self) -> Result<&[u8], Self::Error> {
        let remaining = self.handle_chunk_boundary().await?;
        if remaining == 0 {
            Ok(&[])
        } else {
            let buf = self.raw_body.fill_buf().await.map_err(|e| Error::Network(e.kind()))?;
            let len = buf.len().min(remaining);
            Ok(&buf[..len])
        }
    }

    fn consume(&mut self, amt: usize) {
        let consumed = self.chunk_remaining.consume(amt);
        self.raw_body.consume(consumed);
    }
}
