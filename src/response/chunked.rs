use embedded_io_async::{BufRead, Error as _, ErrorType, Read};

use crate::Error;

#[derive(Clone, Copy, PartialEq, Eq)]
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

        let buf = self.raw_body.fill_buf().await.map_err(|e| Error::Network(e.kind()))?;

        let len = buf.len().min(remaining);

        Ok(&buf[..len])
    }

    fn consume(&mut self, amt: usize) {
        let consumed = self.chunk_remaining.consume(amt);
        self.raw_body.consume(consumed);
    }
}
