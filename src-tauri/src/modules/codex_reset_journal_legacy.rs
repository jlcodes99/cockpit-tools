//! Bounded, byte-addressed traversal of the previous combined reset journal.
use std::{fs::File, io::ErrorKind, path::Path, time::Instant};

use serde::{Deserialize, Serialize};

use super::{message, AccountRule, Pending, Record, Rule};
use crate::modules::background_maintenance::{BATCH_BYTES, BATCH_ROWS, BATCH_TIME};
use crate::modules::legacy_json_input::{Input, ReadError};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub(super) struct Cursor {
    pub offset: u64,
    section: Section,
    next_item: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
enum Section {
    #[default]
    Root,
    AccountRules,
    Pending,
    Evidence,
    Success,
    Records,
}

pub(super) enum Item {
    Revision(u64),
    PublicRule(Rule),
    AccountRule(String, AccountRule),
    Pending(String, Pending),
    Evidence(String, String),
    Success(String, i64),
    SuccessMap,
    Record(u64, Record),
}

pub(super) struct Batch {
    pub cursor: Cursor,
    pub items: Vec<Item>,
    pub stamp: Option<String>,
    pub complete: bool,
}

fn storage_error(error: impl std::fmt::Display) -> String {
    format!("{}: {error}", message("storageError"))
}

pub(super) fn read(
    path: &Path,
    previous_stamp: Option<&str>,
    previous_cursor: &Cursor,
) -> Result<Batch, String> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound && previous_stamp.is_none() => {
            return Ok(Batch {
                cursor: Cursor::default(),
                items: Vec::new(),
                stamp: None,
                complete: true,
            });
        }
        Err(error) => return Err(storage_error(error)),
    };
    let metadata = file.metadata().map_err(storage_error)?;
    let stamp = format!(
        "{}:{:?}",
        metadata.len(),
        metadata.modified().map_err(storage_error)?
    );
    if previous_stamp.is_some_and(|previous| previous != stamp) {
        // Replaying a replaced source could revive entries which were already deleted.
        return Err(storage_error("迁移期间旧用卡记录发生变化"));
    }

    let mut input = Input::new(file, previous_cursor.offset, BATCH_BYTES).map_err(storage_error)?;
    let mut root_closed = false;
    let mut cursor = previous_cursor.clone();
    let mut items = Vec::new();
    let start = Instant::now();
    if cursor.offset == 0 {
        input.expect(b'{').map_err(ReadError::into_message)?;
    }
    let mut complete = false;
    loop {
        match read_item(&mut input, &mut cursor, &mut root_closed) {
            Ok(Some(item)) => items.push(item),
            Ok(None) => {}
            Err(ReadError::Budget) if cursor.offset > previous_cursor.offset => break,
            Err(error) => return Err(error.into_message()),
        }
        cursor.offset = input.offset;
        if matches!(cursor.section, Section::Root) && !cursor.next_item && root_closed {
            complete = true;
            break;
        }
        if items.len() >= BATCH_ROWS || input.budget == 0 || start.elapsed() >= BATCH_TIME {
            break;
        }
    }
    Ok(Batch {
        cursor,
        items,
        stamp: Some(stamp),
        complete,
    })
}

fn read_item(
    input: &mut Input,
    cursor: &mut Cursor,
    root_closed: &mut bool,
) -> Result<Option<Item>, ReadError> {
    let end = if matches!(cursor.section, Section::Records) {
        b']'
    } else {
        b'}'
    };
    if input.peek()? == Some(end) {
        input.take()?;
        if matches!(cursor.section, Section::Root) {
            if input.peek()?.is_some() {
                return Err(ReadError::Format);
            }
            *root_closed = true;
            cursor.next_item = false;
        } else {
            cursor.section = Section::Root;
            cursor.next_item = true;
        }
        return Ok(None);
    }
    if cursor.next_item {
        input.expect(b',')?;
    }

    if matches!(cursor.section, Section::Records) {
        let order = input.offset;
        let row = input.value::<Record>()?;
        cursor.next_item = true;
        return Ok(Some(Item::Record(order, row)));
    }

    let key = input.value::<String>()?;
    input.expect(b':')?;
    let item = match &cursor.section {
        Section::Root => {
            let section = match key.as_str() {
                "accounts" => Some(Section::AccountRules),
                "pending" => Some(Section::Pending),
                "last_evidence" => Some(Section::Evidence),
                "last_automatic_success_ms" if input.peek()? == Some(b'{') => {
                    Some(Section::Success)
                }
                "records" => Some(Section::Records),
                _ => None,
            };
            if let Some(section) = section {
                input.expect(if matches!(section, Section::Records) {
                    b'['
                } else {
                    b'{'
                })?;
                let success_map = matches!(section, Section::Success);
                cursor.section = section;
                cursor.next_item = false;
                return Ok(success_map.then_some(Item::SuccessMap));
            }
            match key.as_str() {
                "revision" => Some(Item::Revision(input.value()?)),
                "public_rule" => Some(Item::PublicRule(input.value()?)),
                "last_automatic_success_ms" => {
                    input.value::<()>()?;
                    None
                }
                _ => {
                    input.value::<serde::de::IgnoredAny>()?;
                    None
                }
            }
        }
        Section::AccountRules => Some(Item::AccountRule(key, input.value()?)),
        Section::Pending => Some(Item::Pending(key, input.value()?)),
        Section::Evidence => Some(Item::Evidence(key, input.value()?)),
        Section::Success => Some(Item::Success(key, input.value()?)),
        Section::Records => unreachable!(),
    };
    cursor.next_item = true;
    Ok(item)
}

impl ReadError {
    fn into_message(self) -> String {
        match self {
            Self::Budget => storage_error("旧用卡记录的单项超出迁移预算"),
            Self::Format => storage_error("旧用卡记录格式无效，原文件已保留"),
            Self::Io(error) => storage_error(error),
        }
    }
}
