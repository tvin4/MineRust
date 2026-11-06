//! Conversion of data pack JSON into NBT.
//!
//! The rules follow the public documentation
//! (<https://minecraft.wiki/w/NBT_format#Conversion_from_JSON>), which is
//! also what the protocol documentation points at for registry entry data:
//! integers pick the smallest NBT integer type that holds them, other
//! numbers become float or double, arrays become byte/int/long arrays when
//! their elements share such a type and a list otherwise, and `null` has no
//! NBT representation.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{NbtError, NbtTag, TAG_END};

impl NbtTag {
    /// Converts a JSON value (as found in data pack files) into the
    /// equivalent NBT tag.
    pub fn from_json(value: &Value) -> Result<Self, NbtError> {
        match value {
            Value::String(text) => Ok(Self::String(text.clone())),
            Value::Bool(flag) => Ok(Self::Byte(i8::from(*flag))),
            Value::Number(number) => number_to_tag(number),
            Value::Null => Err(NbtError::JsonNull),
            Value::Array(items) => array_to_tag(items),
            Value::Object(entries) => {
                let mut compound = BTreeMap::new();
                for (name, item) in entries {
                    compound.insert(name.clone(), Self::from_json(item)?);
                }
                Ok(Self::Compound(compound))
            }
        }
    }
}

/// Converts a JSON number following the documented type selection order.
fn number_to_tag(number: &serde_json::Number) -> Result<NbtTag, NbtError> {
    if let Some(value) = number.as_i64() {
        return Ok(integral_tag(value as i128));
    }
    if let Some(value) = number.as_u64() {
        // Positive integers beyond the long range fall through to the float
        // rules, exactly as documented for numbers outside every integer type.
        return Ok(float_or_double(value as f64));
    }
    let value = number.as_f64().ok_or(NbtError::JsonNumber)?;
    if value == value.trunc() && value >= i64::MIN as f64 && value < i64::MAX as f64 {
        Ok(integral_tag(value as i64 as i128))
    } else {
        Ok(float_or_double(value))
    }
}

/// Picks the smallest NBT integer type that holds an integral value.
fn integral_tag(value: i128) -> NbtTag {
    if (-128..=127).contains(&value) {
        NbtTag::Byte(value as i8)
    } else if (-32768..=32_767).contains(&value) {
        NbtTag::Short(value as i16)
    } else if (i32::MIN as i128..=i32::MAX as i128).contains(&value) {
        NbtTag::Int(value as i32)
    } else {
        NbtTag::Long(value as i64)
    }
}

/// Falls back to float when the value is exactly representable, else double.
fn float_or_double(value: f64) -> NbtTag {
    if (value as f32) as f64 == value {
        NbtTag::Float(value as f32)
    } else {
        NbtTag::Double(value)
    }
}

/// Converts a JSON array; see the module docs for the element type rules.
fn array_to_tag(items: &[Value]) -> Result<NbtTag, NbtError> {
    let mut tags = Vec::with_capacity(items.len());
    for item in items {
        tags.push(NbtTag::from_json(item)?);
    }
    // An empty array yields TAG_END as the element type and lands in the
    // list arm, matching vanilla's empty lists.
    let first_type = tags.first().map_or(TAG_END, NbtTag::type_id);
    match first_type {
        1 => Ok(NbtTag::ByteArray(collect_bytes(tags)?)),
        3 => Ok(NbtTag::IntArray(collect_ints(tags)?)),
        4 => Ok(NbtTag::LongArray(collect_longs(tags)?)),
        other if tags.iter().all(|tag| tag.type_id() == other) => Ok(NbtTag::List(tags)),
        _ => Err(NbtError::MixedJsonArray),
    }
}

fn collect_bytes(tags: Vec<NbtTag>) -> Result<Vec<i8>, NbtError> {
    tags.into_iter()
        .map(|tag| match tag {
            NbtTag::Byte(value) => Ok(value),
            _ => Err(NbtError::MixedJsonArray),
        })
        .collect()
}

fn collect_ints(tags: Vec<NbtTag>) -> Result<Vec<i32>, NbtError> {
    tags.into_iter()
        .map(|tag| match tag {
            NbtTag::Int(value) => Ok(value),
            _ => Err(NbtError::MixedJsonArray),
        })
        .collect()
}

fn collect_longs(tags: Vec<NbtTag>) -> Result<Vec<i64>, NbtError> {
    tags.into_iter()
        .map(|tag| match tag {
            NbtTag::Long(value) => Ok(value),
            _ => Err(NbtError::MixedJsonArray),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn integers_pick_the_smallest_fitting_type() {
        let cases = [
            (json!(0), NbtTag::Byte(0)),
            (json!(1.0), NbtTag::Byte(1)),
            (json!(127), NbtTag::Byte(127)),
            (json!(1.27e2), NbtTag::Byte(127)),
            (json!(-128), NbtTag::Byte(-128)),
            (json!(128), NbtTag::Short(128)),
            (json!(-129), NbtTag::Short(-129)),
            (json!(32767), NbtTag::Short(32767)),
            (json!(32768), NbtTag::Int(32768)),
            (json!(-2147483649i64), NbtTag::Long(-2147483649)),
            (json!(2147483647), NbtTag::Int(2147483647)),
            (json!(2147483648i64), NbtTag::Long(2147483648)),
            (json!(9223372036854775807i64), NbtTag::Long(i64::MAX)),
        ];
        for (value, expected) in cases {
            assert_eq!(
                NbtTag::from_json(&value).expect("convertible"),
                expected,
                "conversion of {value}"
            );
        }
    }

    #[test]
    fn numbers_beyond_long_range_use_float_rules() {
        // 2^63 is integral but out of the long range: exactly representable
        // as a float, so it becomes one.
        let value = json!(9223372036854775808u64);
        assert_eq!(
            NbtTag::from_json(&value).expect("convertible"),
            NbtTag::Float(9223372036854775808.0f64 as f32)
        );
    }

    #[test]
    fn fractional_numbers_become_float_or_double() {
        let cases = [
            (json!(0.5), NbtTag::Float(0.5)),
            (json!(0.25), NbtTag::Float(0.25)),
            (json!(31.75), NbtTag::Float(31.75)),
            (json!(0.6), NbtTag::Double(0.6)),
        ];
        for (value, expected) in cases {
            assert_eq!(
                NbtTag::from_json(&value).expect("convertible"),
                expected,
                "conversion of {value}"
            );
        }
    }

    #[test]
    fn strings_and_booleans_convert() {
        assert_eq!(
            NbtTag::from_json(&json!("hello")).expect("convertible"),
            NbtTag::String("hello".to_owned())
        );
        assert_eq!(
            NbtTag::from_json(&json!(true)).expect("convertible"),
            NbtTag::Byte(1)
        );
        assert_eq!(
            NbtTag::from_json(&json!(false)).expect("convertible"),
            NbtTag::Byte(0)
        );
    }

    #[test]
    fn null_is_rejected() {
        let err = NbtTag::from_json(&json!(null)).expect_err("null");
        assert!(matches!(err, NbtError::JsonNull));
    }

    #[test]
    fn arrays_become_arrays_or_lists_by_element_type() {
        // Byte elements collapse into a byte array: [0, 1, true].
        assert_eq!(
            NbtTag::from_json(&json!([0, 1, true])).expect("convertible"),
            NbtTag::ByteArray(vec![0, 1, 1])
        );
        assert_eq!(
            NbtTag::from_json(&json!([1, 2])).expect("convertible"),
            NbtTag::ByteArray(vec![1, 2])
        );
        // Float elements stay a list of floats: [5e-1, 0.25].
        assert_eq!(
            NbtTag::from_json(&json!([5e-1, 0.25])).expect("convertible"),
            NbtTag::List(vec![NbtTag::Float(0.5), NbtTag::Float(0.25)])
        );
        // Strings stay a list of strings.
        assert_eq!(
            NbtTag::from_json(&json!(["sender", "target"])).expect("convertible"),
            NbtTag::List(vec![
                NbtTag::String("sender".to_owned()),
                NbtTag::String("target".to_owned())
            ])
        );
        // An empty array becomes an empty list.
        assert_eq!(
            NbtTag::from_json(&json!([])).expect("convertible"),
            NbtTag::List(Vec::new())
        );
        // Nested arrays become lists of arrays.
        assert_eq!(
            NbtTag::from_json(&json!([[1]])).expect("convertible"),
            NbtTag::List(vec![NbtTag::ByteArray(vec![1])])
        );
    }

    #[test]
    fn arrays_with_mixed_element_types_are_rejected() {
        // 0 and 1 convert to byte but 128 to short.
        let err = NbtTag::from_json(&json!([0, 1, 128])).expect_err("mixed");
        assert!(matches!(err, NbtError::MixedJsonArray));

        // 0.5 converts to float but 0.6 to double.
        let err = NbtTag::from_json(&json!([0.5, 0.6])).expect_err("mixed");
        assert!(matches!(err, NbtError::MixedJsonArray));
    }

    #[test]
    fn dimension_type_style_object_converts() {
        // Shape of a real data pack dimension_type definition.
        let value = json!({
            "piglin_safe": false,
            "natural": true,
            "ambient_light": 0.0,
            "bed_works": true,
            "min_y": -64,
            "height": 384,
            "logical_height": 384,
            "coordinate_scale": 1.0,
            "ultrawarm": false,
            "has_ceiling": false,
            "has_skylight": true,
            "has_raids": true,
            "infiniburn": "#minecraft:infiniburn_overworld",
            "effects": "minecraft:overworld",
            "clouds": true
        });
        let tag = NbtTag::from_json(&value).expect("convertible");
        let NbtTag::Compound(entries) = &tag else {
            panic!("expected a compound, got {tag:?}");
        };
        assert_eq!(entries["piglin_safe"], NbtTag::Byte(0));
        assert_eq!(entries["natural"], NbtTag::Byte(1));
        // Integral JSON numbers pick the smallest fitting integer type:
        // -64 fits a byte, 384 fits a short.
        assert_eq!(entries["ambient_light"], NbtTag::Byte(0));
        assert_eq!(entries["min_y"], NbtTag::Byte(-64));
        assert_eq!(entries["height"], NbtTag::Short(384));
        assert_eq!(
            entries["effects"],
            NbtTag::String("minecraft:overworld".to_owned())
        );
    }

    #[test]
    fn converted_tags_roundtrip_through_the_network_codec() {
        let value = json!({
            "name": "test",
            "values": [1, 2, 3],
            "nested": {"flag": true, "ratio": 0.5}
        });
        let tag = NbtTag::from_json(&value).expect("convertible");
        let bytes = tag.to_network_bytes().expect("encodable");
        let (decoded, consumed) = NbtTag::decode_network(&bytes).expect("decodable");
        assert_eq!(consumed, bytes.len());
        assert_eq!(decoded, tag);
    }
}
