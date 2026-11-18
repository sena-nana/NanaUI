//! Deterministic binary codec for precompiled `wgpu::naga::Module` IR.
//!
//! Replaces `bincode` with an explicit little-endian + LEB128 varint framing
//! format scoped strictly to NanaUI's checked-in `naga_ir` asset pipeline.

use std::fmt;

use serde::de::{
    self, Deserialize, DeserializeSeed, EnumAccess, IntoDeserializer, MapAccess, SeqAccess,
    VariantAccess, Visitor,
};
use serde::ser::{
    self, Serialize, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant,
    SerializeTuple, SerializeTupleStruct, SerializeTupleVariant,
};

const MAGIC: &[u8; 5] = b"NAGA1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CodecError {
    BadMagic,
    UnexpectedEof,
    VarintOverflow,
    InvalidBool(u8),
    InvalidOptionTag(u8),
    InvalidChar(u32),
    InvalidUtf8,
    LengthOverflow,
    TrailingBytes(usize),
    Unsupported(&'static str),
    Message(String),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => write!(f, "invalid naga IR magic header"),
            Self::UnexpectedEof => write!(f, "unexpected end of naga IR stream"),
            Self::VarintOverflow => write!(f, "varint overflow in naga IR stream"),
            Self::InvalidBool(v) => write!(f, "invalid boolean tag {v} in naga IR stream"),
            Self::InvalidOptionTag(v) => write!(f, "invalid option tag {v} in naga IR stream"),
            Self::InvalidChar(cp) => write!(f, "invalid Unicode scalar {cp:#x} in naga IR stream"),
            Self::InvalidUtf8 => write!(f, "invalid UTF-8 string in naga IR stream"),
            Self::LengthOverflow => write!(f, "length exceeds platform capacity in naga IR stream"),
            Self::TrailingBytes(n) => write!(f, "{n} trailing byte(s) in naga IR stream"),
            Self::Unsupported(kind) => write!(f, "unsupported serde type in naga IR codec: {kind}"),
            Self::Message(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for CodecError {}

impl ser::Error for CodecError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::Message(msg.to_string())
    }
}

impl de::Error for CodecError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::Message(msg.to_string())
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn serialize<T: Serialize>(value: &T) -> Result<Vec<u8>, CodecError> {
    let mut encoder = Encoder {
        out: Vec::from(MAGIC.as_slice()),
    };
    value.serialize(&mut encoder)?;
    Ok(encoder.out)
}

pub(super) fn deserialize<'de, T: Deserialize<'de>>(bytes: &'de [u8]) -> Result<T, CodecError> {
    if bytes.len() < MAGIC.len() || &bytes[..MAGIC.len()] != MAGIC {
        return Err(CodecError::BadMagic);
    }
    let mut decoder = Decoder {
        input: bytes,
        pos: MAGIC.len(),
    };
    let value = T::deserialize(&mut decoder)?;
    let trailing = decoder.remaining();
    if trailing != 0 {
        return Err(CodecError::TrailingBytes(trailing));
    }
    Ok(value)
}

#[cfg_attr(not(test), allow(dead_code))]
struct Encoder {
    out: Vec<u8>,
}

impl Encoder {
    fn push_varint_u64(&mut self, mut v: u64) {
        loop {
            let byte = (v & 0x7F) as u8;
            v >>= 7;
            if v == 0 {
                self.out.push(byte);
                break;
            }
            self.out.push(byte | 0x80);
        }
    }

    fn push_varint_u32(&mut self, v: u32) {
        self.push_varint_u64(u64::from(v));
    }

    fn push_len(&mut self, len: usize) -> Result<(), CodecError> {
        let len = u64::try_from(len).map_err(|_| CodecError::LengthOverflow)?;
        self.push_varint_u64(len);
        Ok(())
    }
}

#[cfg_attr(not(test), allow(dead_code))]
enum SeqEncoder<'a> {
    Known {
        parent: &'a mut Encoder,
    },
    Unknown {
        parent: &'a mut Encoder,
        buf: Encoder,
        count: usize,
    },
}

impl SerializeSeq for SeqEncoder<'_> {
    type Ok = ();
    type Error = CodecError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), CodecError> {
        match self {
            Self::Known { parent } => value.serialize(&mut **parent),
            Self::Unknown { buf, count, .. } => {
                *count = count.checked_add(1).ok_or(CodecError::LengthOverflow)?;
                value.serialize(buf)
            }
        }
    }

    fn end(self) -> Result<(), CodecError> {
        if let Self::Unknown { parent, buf, count } = self {
            parent.push_len(count)?;
            parent.out.extend_from_slice(&buf.out);
        }
        Ok(())
    }
}

#[cfg_attr(not(test), allow(dead_code))]
enum MapEncoder<'a> {
    Known {
        parent: &'a mut Encoder,
    },
    Unknown {
        parent: &'a mut Encoder,
        buf: Encoder,
        count: usize,
    },
}

impl SerializeMap for MapEncoder<'_> {
    type Ok = ();
    type Error = CodecError;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<(), CodecError> {
        match self {
            Self::Known { parent } => key.serialize(&mut **parent),
            Self::Unknown { buf, count, .. } => {
                *count = count.checked_add(1).ok_or(CodecError::LengthOverflow)?;
                key.serialize(buf)
            }
        }
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), CodecError> {
        match self {
            Self::Known { parent } => value.serialize(&mut **parent),
            Self::Unknown { buf, .. } => value.serialize(buf),
        }
    }

    fn end(self) -> Result<(), CodecError> {
        if let Self::Unknown { parent, buf, count } = self {
            parent.push_len(count)?;
            parent.out.extend_from_slice(&buf.out);
        }
        Ok(())
    }
}

impl SerializeTuple for &mut Encoder {
    type Ok = ();
    type Error = CodecError;

    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), CodecError> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), CodecError> {
        Ok(())
    }
}

impl SerializeTupleStruct for &mut Encoder {
    type Ok = ();
    type Error = CodecError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), CodecError> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), CodecError> {
        Ok(())
    }
}

impl SerializeTupleVariant for &mut Encoder {
    type Ok = ();
    type Error = CodecError;

    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), CodecError> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), CodecError> {
        Ok(())
    }
}

impl SerializeStruct for &mut Encoder {
    type Ok = ();
    type Error = CodecError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _key: &'static str,
        value: &T,
    ) -> Result<(), CodecError> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), CodecError> {
        Ok(())
    }
}

impl SerializeStructVariant for &mut Encoder {
    type Ok = ();
    type Error = CodecError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _key: &'static str,
        value: &T,
    ) -> Result<(), CodecError> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<(), CodecError> {
        Ok(())
    }
}

impl<'a> ser::Serializer for &'a mut Encoder {
    type Ok = ();
    type Error = CodecError;
    type SerializeSeq = SeqEncoder<'a>;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = MapEncoder<'a>;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    fn serialize_bool(self, v: bool) -> Result<(), CodecError> {
        self.out.push(u8::from(v));
        Ok(())
    }

    fn serialize_i8(self, v: i8) -> Result<(), CodecError> {
        self.out.push(v as u8);
        Ok(())
    }

    fn serialize_i16(self, v: i16) -> Result<(), CodecError> {
        self.out.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_i32(self, v: i32) -> Result<(), CodecError> {
        self.out.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_i64(self, v: i64) -> Result<(), CodecError> {
        self.out.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_i128(self, v: i128) -> Result<(), CodecError> {
        self.out.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_u8(self, v: u8) -> Result<(), CodecError> {
        self.out.push(v);
        Ok(())
    }

    fn serialize_u16(self, v: u16) -> Result<(), CodecError> {
        self.out.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_u32(self, v: u32) -> Result<(), CodecError> {
        self.out.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_u64(self, v: u64) -> Result<(), CodecError> {
        self.out.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_u128(self, v: u128) -> Result<(), CodecError> {
        self.out.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_f32(self, v: f32) -> Result<(), CodecError> {
        self.out.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_f64(self, v: f64) -> Result<(), CodecError> {
        self.out.extend_from_slice(&v.to_le_bytes());
        Ok(())
    }

    fn serialize_char(self, v: char) -> Result<(), CodecError> {
        self.serialize_u32(v as u32)
    }

    fn serialize_str(self, v: &str) -> Result<(), CodecError> {
        self.serialize_bytes(v.as_bytes())
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<(), CodecError> {
        self.push_len(v.len())?;
        self.out.extend_from_slice(v);
        Ok(())
    }

    fn serialize_none(self) -> Result<(), CodecError> {
        self.out.push(0);
        Ok(())
    }

    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<(), CodecError> {
        self.out.push(1);
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), CodecError> {
        Ok(())
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<(), CodecError> {
        Ok(())
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        variant_index: u32,
        _variant: &'static str,
    ) -> Result<(), CodecError> {
        self.push_varint_u32(variant_index);
        Ok(())
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<(), CodecError> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        variant_index: u32,
        _variant: &'static str,
        value: &T,
    ) -> Result<(), CodecError> {
        self.push_varint_u32(variant_index);
        value.serialize(self)
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<Self::SerializeSeq, CodecError> {
        if let Some(len) = len {
            self.push_len(len)?;
            Ok(SeqEncoder::Known { parent: self })
        } else {
            Ok(SeqEncoder::Unknown {
                parent: self,
                buf: Encoder { out: Vec::new() },
                count: 0,
            })
        }
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, CodecError> {
        Ok(self)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct, CodecError> {
        Ok(self)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, CodecError> {
        self.push_varint_u32(variant_index);
        Ok(self)
    }

    fn serialize_map(self, len: Option<usize>) -> Result<Self::SerializeMap, CodecError> {
        if let Some(len) = len {
            self.push_len(len)?;
            Ok(MapEncoder::Known { parent: self })
        } else {
            Ok(MapEncoder::Unknown {
                parent: self,
                buf: Encoder { out: Vec::new() },
                count: 0,
            })
        }
    }

    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStruct, CodecError> {
        Ok(self)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, CodecError> {
        self.push_varint_u32(variant_index);
        Ok(self)
    }

    fn is_human_readable(&self) -> bool {
        false
    }
}

struct Decoder<'de> {
    input: &'de [u8],
    pos: usize,
}

impl<'de> Decoder<'de> {
    fn remaining(&self) -> usize {
        self.input.len().saturating_sub(self.pos)
    }

    fn read_u8(&mut self) -> Result<u8, CodecError> {
        let byte = *self.input.get(self.pos).ok_or(CodecError::UnexpectedEof)?;
        self.pos += 1;
        Ok(byte)
    }

    fn read_exact(&mut self, len: usize) -> Result<&'de [u8], CodecError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or(CodecError::LengthOverflow)?;
        if end > self.input.len() {
            return Err(CodecError::UnexpectedEof);
        }
        let slice = &self.input[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], CodecError> {
        let slice = self.read_exact(N)?;
        let mut arr = [0u8; N];
        arr.copy_from_slice(slice);
        Ok(arr)
    }

    fn read_varint_u64(&mut self) -> Result<u64, CodecError> {
        let mut out = 0u64;
        let mut shift = 0u32;
        loop {
            let byte = self.read_u8()?;
            let low = u64::from(byte & 0x7F);
            if shift >= 64 || (shift == 63 && low > 1) {
                return Err(CodecError::VarintOverflow);
            }
            out |= low << shift;
            if (byte & 0x80) == 0 {
                return Ok(out);
            }
            shift += 7;
        }
    }

    fn read_varint_u32(&mut self) -> Result<u32, CodecError> {
        let v = self.read_varint_u64()?;
        u32::try_from(v).map_err(|_| CodecError::VarintOverflow)
    }

    fn read_len(&mut self) -> Result<usize, CodecError> {
        let v = self.read_varint_u64()?;
        usize::try_from(v).map_err(|_| CodecError::LengthOverflow)
    }
}

struct SeqDecoder<'a, 'de> {
    decoder: &'a mut Decoder<'de>,
    remaining: usize,
}

impl<'de> SeqAccess<'de> for SeqDecoder<'_, 'de> {
    type Error = CodecError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, CodecError> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        seed.deserialize(&mut *self.decoder).map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        // Bound reservation hints on untrusted or corrupt headers.
        Some(self.remaining.min(1024))
    }
}

struct MapDecoder<'a, 'de> {
    decoder: &'a mut Decoder<'de>,
    remaining: usize,
}

impl<'de> MapAccess<'de> for MapDecoder<'_, 'de> {
    type Error = CodecError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, CodecError> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        seed.deserialize(&mut *self.decoder).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, CodecError> {
        seed.deserialize(&mut *self.decoder)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.remaining.min(1024))
    }
}

struct EnumDecoder<'a, 'de> {
    decoder: &'a mut Decoder<'de>,
}

impl<'a, 'de> EnumAccess<'de> for EnumDecoder<'a, 'de> {
    type Error = CodecError;
    type Variant = Self;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Self::Variant), CodecError> {
        let index = self.decoder.read_varint_u32()?;
        let val = seed.deserialize(index.into_deserializer())?;
        Ok((val, self))
    }
}

impl<'a, 'de> VariantAccess<'de> for EnumDecoder<'a, 'de> {
    type Error = CodecError;

    fn unit_variant(self) -> Result<(), CodecError> {
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value, CodecError> {
        seed.deserialize(self.decoder)
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        visitor.visit_seq(SeqDecoder {
            decoder: self.decoder,
            remaining: len,
        })
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        visitor.visit_seq(SeqDecoder {
            decoder: self.decoder,
            remaining: fields.len(),
        })
    }
}

impl<'de> de::Deserializer<'de> for &mut Decoder<'de> {
    type Error = CodecError;

    fn deserialize_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, CodecError> {
        Err(CodecError::Unsupported("any"))
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        match self.read_u8()? {
            0 => visitor.visit_bool(false),
            1 => visitor.visit_bool(true),
            other => Err(CodecError::InvalidBool(other)),
        }
    }

    fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_i8(self.read_u8()? as i8)
    }

    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_i16(i16::from_le_bytes(self.read_array()?))
    }

    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_i32(i32::from_le_bytes(self.read_array()?))
    }

    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_i64(i64::from_le_bytes(self.read_array()?))
    }

    fn deserialize_i128<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_i128(i128::from_le_bytes(self.read_array()?))
    }

    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_u8(self.read_u8()?)
    }

    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_u16(u16::from_le_bytes(self.read_array()?))
    }

    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_u32(u32::from_le_bytes(self.read_array()?))
    }

    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_u64(u64::from_le_bytes(self.read_array()?))
    }

    fn deserialize_u128<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_u128(u128::from_le_bytes(self.read_array()?))
    }

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_f32(f32::from_le_bytes(self.read_array()?))
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_f64(f64::from_le_bytes(self.read_array()?))
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        let code = u32::from_le_bytes(self.read_array()?);
        let ch = char::from_u32(code).ok_or(CodecError::InvalidChar(code))?;
        visitor.visit_char(ch)
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        let len = self.read_len()?;
        let slice = self.read_exact(len)?;
        let s = std::str::from_utf8(slice).map_err(|_| CodecError::InvalidUtf8)?;
        visitor.visit_borrowed_str(s)
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        self.deserialize_str(visitor)
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        let len = self.read_len()?;
        let slice = self.read_exact(len)?;
        visitor.visit_borrowed_bytes(slice)
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        self.deserialize_bytes(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        match self.read_u8()? {
            0 => visitor.visit_none(),
            1 => visitor.visit_some(self),
            other => Err(CodecError::InvalidOptionTag(other)),
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        visitor.visit_unit()
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        let remaining = self.read_len()?;
        visitor.visit_seq(SeqDecoder {
            decoder: self,
            remaining,
        })
    }

    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        visitor.visit_seq(SeqDecoder {
            decoder: self,
            remaining: len,
        })
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        visitor.visit_seq(SeqDecoder {
            decoder: self,
            remaining: len,
        })
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, CodecError> {
        let remaining = self.read_len()?;
        visitor.visit_map(MapDecoder {
            decoder: self,
            remaining,
        })
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        visitor.visit_seq(SeqDecoder {
            decoder: self,
            remaining: fields.len(),
        })
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, CodecError> {
        visitor.visit_enum(EnumDecoder { decoder: self })
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, CodecError> {
        Err(CodecError::Unsupported("identifier"))
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value, CodecError> {
        Err(CodecError::Unsupported("ignored_any"))
    }

    fn is_human_readable(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    enum VariantSample {
        Unit,
        Newtype(u32),
        Tuple(String, bool),
        Struct { weight: f64, tag: Option<u16> },
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct NewtypeIndex(u32);

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct CompositeSample {
        flag: bool,
        signed: (i8, i16, i32, i64, i128),
        unsigned: (u8, u16, u32, u64, u128),
        floats: (f32, f64),
        letter: char,
        name: String,
        blob: Vec<u8>,
        optional: Option<NewtypeIndex>,
        variants: Vec<VariantSample>,
        table: BTreeMap<String, i32>,
        unit: (),
    }

    fn sample_value() -> CompositeSample {
        let mut table = BTreeMap::new();
        table.insert("alpha".to_owned(), -7);
        table.insert("beta".to_owned(), 42);
        CompositeSample {
            flag: true,
            signed: (-1, -300, -100_000, -9_000_000_000, -170_000_000_000_000),
            unsigned: (7, 500, 100_000, 9_000_000_000, 170_000_000_000_000),
            floats: (3.25, -0.125),
            letter: 'λ',
            name: "naga-module".to_owned(),
            blob: vec![0, 1, 127, 128, 255],
            optional: Some(NewtypeIndex(19)),
            variants: vec![
                VariantSample::Unit,
                VariantSample::Newtype(99),
                VariantSample::Tuple("mix".to_owned(), false),
                VariantSample::Struct {
                    weight: 1.5,
                    tag: None,
                },
            ],
            table,
            unit: (),
        }
    }

    #[test]
    fn roundtrip_and_determinism() {
        let value = sample_value();
        let bytes_a = serialize(&value).expect("serialize sample A");
        let bytes_b = serialize(&value).expect("serialize sample B");
        assert!(bytes_a.starts_with(b"NAGA1"));
        assert_eq!(bytes_a, bytes_b, "serialization must be deterministic");

        let decoded: CompositeSample = deserialize(&bytes_a).expect("deserialize sample");
        assert_eq!(decoded, value);
    }

    #[test]
    fn unsized_seq_and_map_serialize_deterministically() {
        struct UnsizedSeq;
        impl Serialize for UnsizedSeq {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut seq = serializer.serialize_seq(None)?;
                seq.serialize_element(&10u32)?;
                seq.serialize_element(&20u32)?;
                seq.end()
            }
        }
        let encoded = serialize(&UnsizedSeq).expect("serialize unsized seq");
        let decoded: Vec<u32> = deserialize(&encoded).expect("deserialize unsized seq");
        assert_eq!(decoded, vec![10, 20]);
    }

    #[test]
    fn corrupt_and_truncated_inputs_return_errors_without_panicking() {
        let encoded = serialize(&sample_value()).expect("serialize valid sample");

        // Bad or short magic headers.
        assert_eq!(
            deserialize::<CompositeSample>(b""),
            Err(CodecError::BadMagic)
        );
        assert_eq!(
            deserialize::<CompositeSample>(b"NAGA"),
            Err(CodecError::BadMagic)
        );
        assert_eq!(
            deserialize::<CompositeSample>(b"NAGA2rest"),
            Err(CodecError::BadMagic)
        );

        // Every strict prefix truncation must return an error and never panic.
        for cut in MAGIC.len()..encoded.len() {
            assert!(
                deserialize::<CompositeSample>(&encoded[..cut]).is_err(),
                "prefix len {cut} unexpectedly deserialized"
            );
        }

        // Trailing bytes must be rejected.
        let mut with_tail = encoded.clone();
        with_tail.push(0);
        assert_eq!(
            deserialize::<CompositeSample>(&with_tail),
            Err(CodecError::TrailingBytes(1))
        );

        // Corrupt bool/option/char/utf8/varint/huge-length payloads.
        assert_eq!(
            deserialize::<bool>(b"NAGA1\x02"),
            Err(CodecError::InvalidBool(2))
        );
        assert_eq!(
            deserialize::<Option<u8>>(b"NAGA1\x02"),
            Err(CodecError::InvalidOptionTag(2))
        );
        let invalid_char = [b'N', b'A', b'G', b'A', b'1', 0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(
            deserialize::<char>(&invalid_char),
            Err(CodecError::InvalidChar(u32::MAX))
        );
        let invalid_utf8 = [b'N', b'A', b'G', b'A', b'1', 2, 0xFF, 0xFE];
        assert_eq!(
            deserialize::<String>(&invalid_utf8),
            Err(CodecError::InvalidUtf8)
        );

        let varint_overflow = [
            b'N', b'A', b'G', b'A', b'1', 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80,
            0x02,
        ];
        assert_eq!(
            deserialize::<Vec<u8>>(&varint_overflow),
            Err(CodecError::VarintOverflow)
        );

        let huge_vec_len = [
            b'N', b'A', b'G', b'A', b'1', 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F,
        ];
        assert!(deserialize::<Vec<u64>>(&huge_vec_len).is_err());
    }
}
