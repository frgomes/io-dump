extern crate futures;
extern crate bytes;
extern crate tokio_io;

use std::cmp;
use std::fs::File;
use std::io::{Read, Write, BufRead, BufReader, Lines};
use std::path::Path;
use std::time::{Instant, Duration};

use bytes::{Buf, BufMut};
use futures::Poll;
use tokio_io::{AsyncRead, AsyncWrite};
//TODO use tokio_io::codec::{Decoder, Encoder, Framed};
//TODO use tokio_io::split::{ReadHalf, WriteHalf};


pub trait IoDump<T> {
    fn write_block(&mut self, Direction, &[u8]) -> std::io::Result<()>;
}

/// Holds the actual IoDump implementation.
pub struct Holder<T> {
    holder: T,
}

/// Forwards reads and writes upstream.
pub struct Passthru<T> {
    upstream: T,
}

/// Copies all data read from and written to the upstream I/O to a file.
pub struct Logger<T> {
    upstream: T,
    dump: File,
    now: Instant,
}

//--------------------------------------------------------------------------------------------------

impl<T> IoDump<T> {
    pub fn passthru(upstream: T) -> std::io::Result<Holder<Passthru<T>>> {
        Ok(
            Holder {
                holder: Passthru {
                    upstream: upstream,
                }
            })
    }

    pub fn wrapper<P: AsRef<Path>>(upstream: T, path: P) -> std::io::Result<Holder<Logger<T>>> {
        Ok(
            Holder {
                holder: Logger {
                    upstream: upstream,
                    dump: try!(File::create(path)),
                    now: Instant::now(),
                }
            })
    }
}

//--------------------------------------------------------------------------------------------------

impl<T> IoDump<T> for Passthru<T> {
    fn write_block(&mut self, dir: Direction, data: &[u8]) -> std::io::Result<()> {
        Ok(())
    }
}

impl<T> IoDump<T> for Logger<T> {
    fn write_block(&mut self, dir: Direction, data: &[u8]) -> std::io::Result<()> {
        let mut dump = &self.dump;
        let now = self.now;

        if dir == Direction::In {
            try!(write!(dump, "<-  "));
        } else {
            try!(write!(dump, "->  "));
        }

        // Write elapsed time
        let elapsed = millis((Instant::now() - now)) as f64 / 1000.0;
        try!(write!(dump, "{:.*}s  {} bytes", 3, elapsed, data.len()));

        // Write newline
        try!(write!(dump, "\n"));

        let mut pos = 0;

        while pos < data.len() {
            let end = cmp::min(pos + LINE, data.len());

            //try!(self.write_data_line(&data[pos..end]));
            let line = &data[pos..end];

            // First write binary
            for i in 0..LINE {
                if i >= line.len() {
                    try!(write!(dump, "   "));
                } else {
                    try!(write!(dump, "{:02X} ", line[i]));
                }
            }

            // Write some spacing for the ascii
            try!(write!(dump, "    "));

            for &byte in line.iter() {
                match byte {
                    0 => try!(write!(dump, "\\0")),
                    9 => try!(write!(dump, "\\t")),
                    10 => try!(write!(dump, "\\n")),
                    13 => try!(write!(dump, "\\r")),
                    32...126 => {
                        try!(dump.write(&[b' ', byte]));
                    }
                    _ => try!(write!(dump, "\\?")),
                }
            }

            write!(dump, "\n");

            pos = end;
        }

        try!(write!(dump, "\n"));

        Ok(())
    }
}

//--------------------------------------------------------------------------------------------------

impl<T> IoDump<T> for Holder<Passthru<T>> {
    fn write_block(&mut self, dir: Direction, data: &[u8]) -> std::io::Result<()> {
        self.holder.write_block(dir, data)
    }
}

impl<T> IoDump<T> for Holder<Logger<T>> {
    fn write_block(&mut self, dir: Direction, data: &[u8]) -> std::io::Result<()> {
        self.holder.write_block(dir, data)
    }
}

//--------------------------------------------------------------------------------------------------

impl<T: Read> Read for Holder<Passthru<T>> {
    fn read(&mut self, dst: &mut [u8]) -> std::io::Result<usize> {
        let n = try!(self.holder.upstream.read(dst));
        Ok(n)
    }
}

impl<T: Write> Write for Holder<Passthru<T>> {
    fn write(&mut self, src: &[u8]) -> std::io::Result<usize> {
        let n = try!(self.holder.upstream.write(src));
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        try!(self.holder.upstream.flush());
        Ok(())
    }
}

impl<T: AsyncRead> AsyncRead for Holder<Passthru<T>> {
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        self.holder.upstream.prepare_uninitialized_buffer(buf)
    }

    fn read_buf<B: BufMut>(&mut self, buf: &mut B) -> Poll<usize, std::io::Error>
        where Self: Sized,
    {
        self.holder.upstream.read_buf(buf)
    }

    //TODO fn framed<T: Encoder + Decoder>(self, codec: T) -> Framed<Self, T> {
    //TODO     self.holder.upstream.framed(codec)
    //TODO }

    //TODO fn split(self) -> (ReadHalf<Self>, WriteHalf<Self>) {
    //TODO     self.holder.upstream.split()
    //TODO }
}

impl<T: AsyncWrite> AsyncWrite for Holder<Passthru<T>> {
    fn shutdown(&mut self) -> Poll<(), std::io::Error> {
        self.holder.upstream.shutdown()
    }

    fn write_buf<B: Buf>(&mut self, buf: &mut B) -> Poll<usize, std::io::Error> {
        self.holder.upstream.write_buf(buf)
    }
}

//--------------------------------------------------------------------------------------------------

impl<T: Read> Read for Holder<Logger<T>> {
    fn read(&mut self, dst: &mut [u8]) -> std::io::Result<usize> {
        let n = try!(self.holder.upstream.read(dst));
        try!(self.holder.write_block(Direction::Out, &dst[0..n]));
        Ok(n)
    }
}

impl<T: Write> Write for Holder<Logger<T>> {
    fn write(&mut self, src: &[u8]) -> std::io::Result<usize> {
        let n = try!(self.holder.upstream.write(src));
        try!(self.holder.write_block(Direction::In, &src[0..n]));
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        try!(self.holder.upstream.flush());
        Ok(())
    }
}

impl<T: AsyncRead> AsyncRead for Holder<Logger<T>> {
    unsafe fn prepare_uninitialized_buffer(&self, buf: &mut [u8]) -> bool {
        self.holder.upstream.prepare_uninitialized_buffer(buf)
    }

    fn read_buf<B: BufMut>(&mut self, buf: &mut B) -> Poll<usize, std::io::Error>
        where Self: Sized,
    {
        //TODO: unable to wait for completion? unable to call write_block ?
        self.holder.upstream.read_buf(buf)
    }

    //TODO fn framed<T: Encoder + Decoder>(self, codec: T) -> Framed<Self, T> {
    //TODO     self.holder.upstream.framed(codec)
    //TODO }

    //TODO fn split(self) -> (ReadHalf<Self>, WriteHalf<Self>) {
    //TODO     self.holder.upstream.split()
    //TODO }
}

impl<T: AsyncWrite> AsyncWrite for Holder<Logger<T>> {
    fn shutdown(&mut self) -> Poll<(), std::io::Error> {
        self.holder.upstream.shutdown()
    }

    fn write_buf<B: Buf>(&mut self, buf: &mut B) -> Poll<usize, std::io::Error> {
        //TODO: unable to wait for completion? unable to call write_block ?
        self.holder.upstream.write_buf(buf)
    }
}

//--------------------------------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Direction {
    In,
    Out,
}

//--------------------------------------------------------------------------------------------------

const LINE: usize = 25;
const NANOS_PER_MILLI: u32 = 1_000_000;
const MILLIS_PER_SEC: u64 = 1_000;

/// Convert a `Duration` to milliseconds, rounding up and saturating at
/// `u64::MAX`.
///
/// The saturating is fine because `u64::MAX` milliseconds are still many
/// million years.
pub fn millis(duration: Duration) -> u64 {
    // Round up.
    let millis = (duration.subsec_nanos() + NANOS_PER_MILLI - 1) / NANOS_PER_MILLI;
    duration.as_secs().saturating_mul(MILLIS_PER_SEC).saturating_add(millis as u64)
}





//--------------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct Block {
    head: Head,
    data: Vec<u8>,
}

#[derive(Debug)]
struct Head {
    direction: Direction,
    elapsed: Duration,
}

impl Block {
    pub fn direction(&self) -> Direction {
        self.head.direction
    }

    pub fn elapsed(&self) -> Duration {
        self.head.elapsed
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

/*
 *
 * ===== impl Dump =====
 *
 */

pub struct Dump {
    lines: Lines<BufReader<File>>,
}

impl Dump {
    pub fn open<P: AsRef<Path>>(path: P) -> std::io::Result<Dump> {
        let dump = try!(File::open(path));
        let dump = BufReader::new(dump);
        Ok(Dump { lines: dump.lines() })
    }

    fn read_block(&mut self) -> std::io::Result<Option<Block>> {
        loop {
            let head = match self.lines.next() {
                Some(Ok(line)) => line,
                Some(Err(e)) => return Err(e),
                None => return Ok(None),
            };

            let head: Vec<String> = head
                .split(|v| v == ' ')
                .filter(|v| !v.is_empty())
                .map(|v| v.into())
                .collect();

            if head.len() == 0 || head[0] == "//" {
                continue;
            }

            assert_eq!(4, head.len());

            let dir = match &head[0][..] {
                "<-" => Direction::In,
                "->" => Direction::Out,
                _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid direction format")),
            };

            let elapsed: f64 = {
                let s = &head[1];
                s[..s.len()-1].parse().unwrap()
            };

            // Do nothing w/ bytes for now

            // ready body
            let mut data = vec![];

            loop {
                let line = match self.lines.next() {
                    Some(Ok(line)) => line,
                    Some(Err(e)) => return Err(e),
                    None => "".into(),
                };

                if line.is_empty() {
                    return Ok(Some(Block {
                        head: Head {
                            direction: dir,
                            elapsed: Duration::from_millis((elapsed * 1000.0) as u64),
                        },
                        data: data,
                    }));
                }

                let mut pos = 0;

                loop {
                    let c = &line[pos..pos+2];

                    if c == "  " {
                        break;
                    }

                    let byte = match u8::from_str_radix(c, 16) {
                        Ok(byte) => byte,
                        Err(_) => return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "could not parse byte")),
                    };

                    data.push(byte);

                    pos += 3;
                }
            }
        }
    }
}

impl Iterator for Dump {
    type Item = Block;

    fn next(&mut self) -> Option<Block> {
        self.read_block().unwrap()
    }
}


