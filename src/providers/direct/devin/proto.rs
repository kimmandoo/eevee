//! Small wire codec for the fields used by Devin's native API. Unknown ordinary
//! protobuf fields are skipped, never interpreted as host instructions.
#[derive(Default)]
pub(super) struct Encode(pub Vec<u8>);
impl Encode {
    fn varint(&mut self, mut value: u64) {
        while value >= 128 {
            self.0.push(value as u8 | 128);
            value >>= 7;
        }
        self.0.push(value as u8);
    }
    pub fn number(&mut self, field: u32, value: u64) {
        self.varint(u64::from(field) << 3);
        self.varint(value);
    }
    pub fn bytes(&mut self, field: u32, value: &[u8]) {
        self.varint((u64::from(field) << 3) | 2);
        self.varint(value.len() as u64);
        self.0.extend_from_slice(value);
    }
    pub fn text(&mut self, field: u32, value: &str) {
        if !value.is_empty() {
            self.bytes(field, value.as_bytes());
        }
    }
    pub fn double(&mut self, field: u32, value: f64) {
        self.varint((u64::from(field) << 3) | 1);
        self.0.extend_from_slice(&value.to_le_bytes());
    }
}
#[derive(Clone, Copy)]
pub(super) enum Value<'a> {
    Number(u64),
    Bytes(&'a [u8]),
    Fixed,
}
#[derive(Clone, Copy)]
pub(super) struct Message<'a>(pub &'a [u8]);
impl<'a> Message<'a> {
    pub fn fields(self) -> Fields<'a> {
        Fields(self.0)
    }
    pub fn bytes(self, field: u32) -> Result<Option<&'a [u8]>, String> {
        let mut found = None;
        for item in self.fields() {
            let (id, value) = item?;
            if id == field {
                let Value::Bytes(bytes) = value else {
                    return Err("Devin protobuf field has wrong wire type".into());
                };
                if found.replace(bytes).is_some() {
                    return Err("Duplicate Devin protobuf scalar".into());
                }
            }
        }
        Ok(found)
    }
    pub fn text(self, field: u32) -> Result<&'a str, String> {
        std::str::from_utf8(self.bytes(field)?.unwrap_or_default())
            .map_err(|_| "Invalid Devin protobuf UTF-8".into())
    }
    pub fn number(self, field: u32) -> Result<Option<u64>, String> {
        let mut found = None;
        for item in self.fields() {
            let (id, value) = item?;
            if id == field {
                let Value::Number(number) = value else {
                    return Err("Devin protobuf field has wrong wire type".into());
                };
                if found.replace(number).is_some() {
                    return Err("Duplicate Devin protobuf scalar".into());
                }
            }
        }
        Ok(found)
    }
    pub fn message(self, field: u32) -> Result<Self, String> {
        Ok(Self(self.bytes(field)?.unwrap_or_default()))
    }
}
pub(super) struct Fields<'a>(&'a [u8]);
fn varint(input: &mut &[u8]) -> Result<u64, String> {
    let mut value = 0u64;
    for shift in (0..70).step_by(7) {
        let (&byte, rest) = input
            .split_first()
            .ok_or("Truncated Devin protobuf varint")?;
        *input = rest;
        if shift == 63 && byte > 1 {
            return Err("Overflowing Devin protobuf varint".into());
        }
        value |= u64::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return Ok(value);
        }
    }
    Err("Overflowing Devin protobuf varint".into())
}
impl<'a> Iterator for Fields<'a> {
    type Item = Result<(u32, Value<'a>), String>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_empty() {
            return None;
        }
        let result = (|| {
            let key = varint(&mut self.0)?;
            let field = u32::try_from(key >> 3).map_err(|_| "Invalid Devin protobuf field")?;
            if field == 0 || field >= (1 << 29) {
                return Err("Invalid Devin protobuf field".into());
            }
            let value = match key & 7 {
                0 => Value::Number(varint(&mut self.0)?),
                wire @ (1 | 2 | 5) => {
                    let count = match wire {
                        1 => 8,
                        5 => 4,
                        _ => usize::try_from(varint(&mut self.0)?)
                            .map_err(|_| "Oversized Devin protobuf field")?,
                    };
                    let bytes = self
                        .0
                        .get(..count)
                        .ok_or("Truncated Devin protobuf field")?;
                    self.0 = &self.0[count..];
                    if wire == 2 {
                        Value::Bytes(bytes)
                    } else {
                        Value::Fixed
                    }
                }
                _ => return Err("Unsupported Devin protobuf wire type".into()),
            };
            Ok((field, value))
        })();
        if result.is_err() {
            self.0 = &[];
        }
        Some(result)
    }
}
