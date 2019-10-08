use ton_abi_core::types::{
    bitstring_to_be_bytes, get_next_bits_from_chain, Bitstring,
    ABIDeserialized, ABISerialized, DeserializationError, Dint, Duint,
};
use types::int::Int;
use types::uint::Uint;
use {Param, ParamType};
use serde_json;
use std::sync::Arc;
use super::*;

use num_bigint::{BigInt, BigUint};
use tvm::stack::{CellData, BuilderData, SliceData};
use tvm::stack::dictionary::{HashmapE, HashmapType};
use tvm::block::BlockResult;
use tvm::block::types::Grams;

impl TokenValue {
    /// Deserializes value from `SliceData` to `TokenValue`
    pub fn read_from(
        param_type: &ParamType,
        mut cursor: SliceData,
    ) -> Result<(Self, SliceData), DeserializationError> {
        match param_type {
            ParamType::Unknown => Err(DeserializationError::with(cursor)),
            ParamType::Uint(size) => Self::read_uint(*size, cursor),
            ParamType::Int(size) => Self::read_int(*size, cursor),
            ParamType::Dint => {
                let (dint, cursor) = Dint::read_from(cursor)?;
                Ok((TokenValue::Dint(dint), cursor))
            }
            ParamType::Duint => {
                let (duint, cursor) = Duint::read_from(cursor)?;
                Ok((TokenValue::Duint(duint), cursor))
            }
            ParamType::Bool => {
                let (b, cursor) = bool::read_from(cursor)?;
                Ok((TokenValue::Bool(b), cursor))
            }
            ParamType::Tuple(tuple_params) => Self::read_tuple(tuple_params, cursor),
            ParamType::Array(param_type) => Self::read_array(&param_type, cursor),
            ParamType::FixedArray(param_type, size) => {
                Self::read_fixed_array(&param_type, *size, cursor)
            }
            ParamType::Bits(size) => Self::read_bits(*size, cursor),
            ParamType::Bitstring => {
                let (bitstring, cursor) = Bitstring::read_from(cursor)?;
                Ok((TokenValue::Bitstring(bitstring), cursor))
            }
            ParamType::Cell => Self::read_cell(cursor)
                .map(|(cell, cursor)| (TokenValue::Cell(cell), cursor)),
            ParamType::Map(key_type, value_type) => Self::read_hashmap(key_type, value_type, cursor),
            ParamType::Address => {
                let original = cursor.clone();
                <MsgAddress as tvm::block::Deserializable>::construct_from(&mut cursor)
                    .map(|address| (TokenValue::Address(address), cursor))
                    .map_err(|_| DeserializationError::with(original))
            }
            ParamType::Bytes => Self::read_bytes(None, cursor),
            ParamType::FixedBytes(size) => Self::read_bytes(Some(*size), cursor),
            ParamType::Gram => {
                let original = cursor.clone();
                <Grams as tvm::block::Deserializable>::construct_from(&mut cursor.clone())
                    .map(|gram: Grams| (TokenValue::Gram(gram.value().to_biguint().unwrap()), cursor))
                    .map_err(|_| DeserializationError::with(original))
            }
        }
    }

    fn read_uint(
        size: usize,
        cursor: SliceData,
    ) -> Result<(Self, SliceData), DeserializationError> {
        let (bitstring, cursor) = get_next_bits_from_chain(cursor, size)?;

        let vec = bitstring_to_be_bytes(bitstring, false);

        let result = Uint {
            number: BigUint::from_bytes_be(&vec),
            size: size,
        };

        Ok((TokenValue::Uint(result), cursor))
    }

    fn read_int(size: usize, cursor: SliceData) -> Result<(Self, SliceData), DeserializationError> {
        let (bitstring, cursor) = get_next_bits_from_chain(cursor, size)?;

        let vec = bitstring_to_be_bytes(bitstring, true);

        let result = Int {
            number: BigInt::from_signed_bytes_be(&vec),
            size: size,
        };

        Ok((TokenValue::Int(result), cursor))
    }

    fn read_tuple(
        tuple_params: &[Param],
        cursor: SliceData,
    ) -> Result<(Self, SliceData), DeserializationError> {
        let mut tokens = Vec::new();
        let mut cursor = cursor;
        for param in tuple_params {
            let (token_value, new_cursor) = TokenValue::read_from(&param.kind, cursor)?;
            tokens.push(Token {
                name: param.name.clone(),
                value: token_value,
            });
            cursor = new_cursor;
        }
        Ok((TokenValue::Tuple(tokens), cursor))
    }

    fn read_bits(
        size: usize,
        cursor: SliceData,
    ) -> Result<(Self, SliceData), DeserializationError> {
        let (token, cursor) = Self::read_fixed_array(&ParamType::Bool, size, cursor)?;

        if let TokenValue::FixedArray(array) = token {
            let bitstring = array.iter().fold(Bitstring::new(), |mut bitstring, token| {
                if let TokenValue::Bool(b) = token {
                    bitstring.append_bit_bool(*b);
                    bitstring
                } else {
                    unreachable!();
                }
            });

            Ok((TokenValue::Bits(bitstring), cursor))
        } else {
            unreachable!();
        }
    }

    fn read_array_from_branch(
        param_type: &ParamType,
        cursor: SliceData,
    ) -> Result<(Vec<Self>, SliceData), DeserializationError> {
        let mut cursor = cursor;

        if cursor.remaining_references() == 0 {
            return Err(DeserializationError::with(cursor));
        }

        let mut array_cursor: SliceData = cursor.checked_drain_reference().unwrap().into();
        let mut result = vec![];

        while array_cursor.remaining_references() != 0 || array_cursor.remaining_bits() != 0 {
            let (token, new_cursor) = Self::read_from(param_type, array_cursor)?;
            array_cursor = new_cursor;
            result.push(token);
        }

        if array_cursor.remaining_references() != 0 || array_cursor.remaining_bits() != 0 {
            return Err(DeserializationError::with(array_cursor));
        }

        Ok((result, cursor))
    }

    fn read_array_from_map(
        param_type: &ParamType,
        cursor: SliceData,
    ) -> Result<(Vec<Self>, SliceData), DeserializationError> {

        let (size, cursor) = <u32>::read_from(cursor)?;
        let (slice, cursor) = <HashmapE>::read_from(cursor)?;
        let map = HashmapE::with_data(32, slice);

        let mut result = vec![];
        for i in 0..size {
            let mut index = BuilderData::new();
            index = (i as u32).prepend_to(index);

            let item_slice = map.get(index.into())
                .map_err(|_| DeserializationError::with(cursor.clone()))?
                .ok_or(DeserializationError::with(cursor.clone()))?;

            let (token, item_slice) = Self::read_from(param_type, item_slice)?;

            if item_slice.remaining_references() != 0 || item_slice.remaining_bits() != 0 {
                return Err(DeserializationError::with(item_slice));
            }

            result.push(token);
        }

        Ok((result, cursor))
    }

    fn read_array(
        param_type: &ParamType,
        cursor: SliceData,
    ) -> Result<(Self, SliceData), DeserializationError> {
        let (flag, cursor) = <(bool, bool)>::read_from(cursor)?;

        match flag {
            (false, false) => {
                let (result, cursor) = Self::read_array_from_branch(param_type, cursor)?;

                Ok((TokenValue::Array(result), cursor))
            }
            (false, true) => {
                let (result, cursor) = Self::read_array_from_map(param_type, cursor)?;

                Ok((TokenValue::Array(result), cursor))
            }
            (true, false) => {
                let (size, mut cursor) = <u8>::read_from(cursor)?;
                let mut result = vec![];

                for _ in 0..size {
                    let (token, new_cursor) = Self::read_from(param_type, cursor)?;
                    cursor = new_cursor;
                    result.push(token);
                }

                Ok((TokenValue::Array(result), cursor))
            }
            _ => Err(DeserializationError::with(cursor)),
        }
    }

    fn read_fixed_array(
        param_type: &ParamType,
        size: usize,
        cursor: SliceData,
    ) -> Result<(Self, SliceData), DeserializationError> {
        let (flag, mut cursor) = <(bool, bool)>::read_from(cursor)?;

        match flag {
            (false, false) => {
                let (result, cursor) = Self::read_array_from_branch(param_type, cursor)?;

                Ok((TokenValue::FixedArray(result), cursor))
            }
            (true, false) => {
                let mut result = vec![];

                for _ in 0..size {
                    let (token, new_cursor) = Self::read_from(param_type, cursor)?;
                    cursor = new_cursor;
                    result.push(token);
                }

                Ok((TokenValue::FixedArray(result), cursor))
            }
            _ => Err(DeserializationError::with(cursor)),
        }
    }

    fn read_cell(mut cursor: SliceData) -> Result<(Arc<CellData>, SliceData), DeserializationError> {
        let original = cursor.clone();
        let cell = match cursor.remaining_references() {
            0 => return Err(DeserializationError::with(original)),
            1 => {
                cursor = SliceData::from(cursor.reference(0).unwrap());
                cursor.checked_drain_reference()
                    .map_err(|_| DeserializationError::with(original))?
            }
            _ => cursor.checked_drain_reference().unwrap()
        };
        Ok((cell.clone(), cursor))
    }

    fn read_hashmap(key_type: &ParamType, value_type: &ParamType, cursor: SliceData)
    -> Result<(Self, SliceData), DeserializationError> {
        let original = cursor.clone();
        let (flag, mut cursor) = <bool>::read_from(cursor)?;
        let mut new_map = HashMap::new();
        if flag {
            let cell = cursor.checked_drain_reference()
                .map_err(|_| DeserializationError::with(original.clone()))?;
            let hashmap = HashmapE::with_hashmap(key_type.bit_len(), Some(cell));
            hashmap.iterate(&mut |key, value| -> BlockResult<bool> {
                let key = Self::read_from(key_type, key).unwrap().0;
                let value = Self::read_from(value_type, value).unwrap().0;
                let key = serde_json::to_string(&key).unwrap();
                new_map.insert(key, value);
                Ok(true)
            }).map_err(|_| DeserializationError::with(original))?;
        }
        Ok((TokenValue::Map(value_type.clone(), new_map), cursor))
    }

    fn read_bytes(size: Option<usize>, cursor: SliceData)
    -> Result<(Self, SliceData), DeserializationError> {
        let original = cursor.clone();
        let (mut cell, cursor) = Self::read_cell(cursor)?;

        let mut data = vec![];
        loop {
            data.extend_from_slice(cell.data());
            cell = match cell.reference(0) {
                Ok(cell) => cell.clone(),
                Err(_) => break
            };
        }
        match size {
            Some(size) => if size == data.len() {
                Ok((TokenValue::FixedBytes(data), cursor))
            } else {
                Err(DeserializationError::with(original))
            }
            None => Ok((TokenValue::Bytes(data), cursor))
        }
    }
}
