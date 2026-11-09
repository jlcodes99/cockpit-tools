use serde::{
    de::{value::MapAccessDeserializer, DeserializeOwned, MapAccess, Visitor},
    Deserialize, Deserializer,
};
use std::io::{self, Read};
use std::{fmt, marker::PhantomData};

const CHUNK_BYTES: usize = 64 * 1024;

pub(in crate::modules) struct Records<R, F> {
    input: R,
    on_chunk: F,
    buffer: Vec<u8>,
    start: usize,
    end: usize,
    // Up to three bytes of an unfinished UTF-8 code point follow end.
    tail: usize,
    pub(in crate::modules) position: u64,
}

pub(in crate::modules) struct Record<T> {
    pub value: serde_json::Result<T>,
    pub complete: bool,
    pub blank: bool,
}

// serde's derived structs also accept positional arrays. A rollout record and
// its metadata are JSON objects, so require map input before applying a projection.
pub(in crate::modules) struct Object<T>(pub T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor<T>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjectVisitor<T> {
            type Value = Object<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a rollout JSON object")
            }

            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
                T::deserialize(MapAccessDeserializer::new(map)).map(Object)
            }
        }
        deserializer.deserialize_map(ObjectVisitor(PhantomData))
    }
}

impl<R: Read, F: FnMut(&[u8]) -> io::Result<()>> Records<R, F> {
    pub fn new(input: R, on_chunk: F) -> Self {
        Self {
            input,
            on_chunk,
            buffer: vec![0; CHUNK_BYTES],
            start: 0,
            end: 0,
            tail: 0,
            position: 0,
        }
    }

    fn refill(&mut self) -> io::Result<()> {
        self.buffer.copy_within(self.end..self.end + self.tail, 0);
        let mut length = self.tail;
        self.start = 0;
        self.end = 0;
        self.tail = 0;
        loop {
            let count = match self.input.read(&mut self.buffer[length..]) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                result => result?,
            };
            if count == 0 && length == 0 {
                return Ok(());
            }
            length += count;
            self.end = match std::str::from_utf8(&self.buffer[..length]) {
                Ok(_) => length,
                Err(error) if error.error_len().is_none() && count != 0 => error.valid_up_to(),
                Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidData, error)),
            };
            if self.end != 0 {
                self.tail = length - self.end;
                return (self.on_chunk)(&self.buffer[..self.end]);
            }
        }
    }

    pub fn next<T: DeserializeOwned>(&mut self) -> io::Result<Option<Record<T>>> {
        if self.start == self.end {
            self.refill()?;
        }
        let chunk = &self.buffer[self.start..self.end];
        if chunk.is_empty() {
            return Ok(None);
        }
        if let Some(newline) = chunk.iter().position(|&byte| byte == b'\n') {
            let line = &chunk[..=newline];
            self.start += line.len();
            self.position += line.len() as u64;
            return Ok(Some(Record {
                value: serde_json::from_slice::<Object<T>>(line).map(|object| object.0),
                complete: true,
                blank: line.iter().all(u8::is_ascii_whitespace),
            }));
        }

        // Long records go directly through the deserializer. Ignored fields are
        // scanned for JSON syntax without allocating their string or array values.
        let end = self.end;
        let mut line = RecordReader {
            records: self,
            end,
            newline: false,
            complete: false,
            blank: true,
        };
        let value = serde_json::from_reader::<_, Object<T>>(&mut line).map(|object| object.0);
        let value = match value {
            Err(error) if error.is_io() => return Err(error.into()),
            value => value,
        };
        // A syntax error can leave unread bytes in the record. Consume through
        // the delimiter so callers that skip invalid lines can resume.
        io::copy(&mut line, &mut io::sink())?;
        Ok(Some(Record {
            value,
            complete: line.complete,
            blank: line.blank,
        }))
    }
}

struct RecordReader<'a, R, F> {
    records: &'a mut Records<R, F>,
    end: usize,
    newline: bool,
    complete: bool,
    blank: bool,
}

impl<R: Read, F: FnMut(&[u8]) -> io::Result<()>> Read for RecordReader<'_, R, F> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.complete || output.is_empty() {
            return Ok(0);
        }
        if self.records.start == self.end {
            self.records.refill()?;
            let chunk = &self.records.buffer[..self.records.end];
            self.end = chunk.len();
            if let Some(newline) = chunk.iter().position(|&byte| byte == b'\n') {
                self.end = newline + 1;
                self.newline = true;
            }
        }
        let count = output.len().min(self.end - self.records.start);
        let next = self.records.start + count;
        let bytes = &self.records.buffer[self.records.start..next];
        output[..count].copy_from_slice(bytes);
        self.blank = self.blank && bytes.iter().all(u8::is_ascii_whitespace);
        self.records.start = next;
        self.records.position += count as u64;
        self.complete = self.newline && next == self.end;
        Ok(count)
    }
}
