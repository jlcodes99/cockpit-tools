//! Read individual values from a JSON stream within an explicit byte budget.
use std::{
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub(super) enum ReadError {
    Budget,
    Format,
    Io(std::io::Error),
}

impl From<std::io::Error> for ReadError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

pub(super) struct Input {
    reader: BufReader<File>,
    pub(super) offset: u64,
    pub(super) budget: u64,
}

impl Input {
    pub(super) fn new(file: File, offset: u64, budget: u64) -> Result<Self, std::io::Error> {
        let mut reader = BufReader::new(file);
        reader.seek(SeekFrom::Start(offset))?;
        Ok(Self {
            reader,
            offset,
            budget,
        })
    }

    fn raw(&mut self) -> Result<Option<u8>, ReadError> {
        Ok(self.reader.fill_buf()?.first().copied())
    }

    pub(super) fn take(&mut self) -> Result<Option<u8>, ReadError> {
        let byte = self.raw()?;
        if byte.is_some() {
            if self.budget == 0 {
                return Err(ReadError::Budget);
            }
            self.reader.consume(1);
            self.offset += 1;
            self.budget -= 1;
        }
        Ok(byte)
    }

    pub(super) fn peek(&mut self) -> Result<Option<u8>, ReadError> {
        while self.raw()?.is_some_and(|byte| byte.is_ascii_whitespace()) {
            self.take()?;
        }
        self.raw()
    }

    pub(super) fn expect(&mut self, expected: u8) -> Result<(), ReadError> {
        if self.peek()? != Some(expected) {
            return Err(ReadError::Format);
        }
        self.take()?;
        Ok(())
    }

    pub(super) fn value<T: DeserializeOwned>(&mut self) -> Result<T, ReadError> {
        let first = self.peek()?.ok_or(ReadError::Format)?;
        let mut bytes = vec![self.take()?.ok_or(ReadError::Format)?];
        let mut depth: usize = usize::from(matches!(first, b'{' | b'['));
        let mut string = first == b'"';
        let mut escaped = false;
        while let Some(byte) = self.raw()? {
            if !string
                && depth == 0
                && (byte.is_ascii_whitespace() || matches!(byte, b':' | b',' | b'}' | b']'))
            {
                break;
            }
            bytes.push(self.take()?.ok_or(ReadError::Format)?);
            if string {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    string = false;
                    if depth == 0 {
                        break;
                    }
                }
            } else {
                match byte {
                    b'"' => string = true,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth = depth.checked_sub(1).ok_or(ReadError::Format)?;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
        }
        serde_json::from_slice(&bytes).map_err(|_| ReadError::Format)
    }
}

/// Resume traversal of an array inside a legacy JSON object by byte offset.
#[derive(Clone, Default, PartialEq, Deserialize, Serialize)]
pub(super) struct ArrayCursor {
    pub(super) offset: u64,
    #[serde(alias = "in_tasks")]
    in_array: bool,
    next_item: bool,
}

pub(super) fn read_array_batch<T: DeserializeOwned>(
    input: &mut Input,
    cursor: &mut ArrayCursor,
    field: &str,
    max_rows: usize,
    time_budget: std::time::Duration,
) -> Result<(Vec<T>, bool), ReadError> {
    let started = std::time::Instant::now();
    let offset = cursor.offset;
    let mut rows = Vec::new();
    let mut complete = false;
    if offset == 0 {
        input.expect(b'{')?;
    }
    loop {
        match read_array_item(input, cursor, field, &mut complete) {
            Ok(Some(item)) => rows.push(item),
            Ok(None) => {}
            Err(ReadError::Budget) if cursor.offset > offset => break,
            Err(cause) => return Err(cause),
        }
        cursor.offset = input.offset;
        if complete
            || rows.len() >= max_rows
            || input.budget == 0
            || started.elapsed() >= time_budget
        {
            break;
        }
    }
    Ok((rows, complete))
}

fn read_array_item<T: DeserializeOwned>(
    input: &mut Input,
    cursor: &mut ArrayCursor,
    field: &str,
    complete: &mut bool,
) -> Result<Option<T>, ReadError> {
    if input.peek()? == Some(if cursor.in_array { b']' } else { b'}' }) {
        input.take()?;
        if cursor.in_array {
            cursor.in_array = false;
            cursor.next_item = true;
        } else {
            if input.peek()?.is_some() {
                return Err(ReadError::Format);
            }
            *complete = true;
        }
        return Ok(None);
    }
    if cursor.next_item {
        input.expect(b',')?;
    }
    if cursor.in_array {
        let item = input.value()?;
        cursor.next_item = true;
        return Ok(Some(item));
    }
    let key = input.value::<String>()?;
    input.expect(b':')?;
    if key == field {
        input.expect(b'[')?;
        cursor.in_array = true;
        cursor.next_item = false;
    } else {
        input.value::<serde::de::IgnoredAny>()?;
        cursor.next_item = true;
    }
    Ok(None)
}
