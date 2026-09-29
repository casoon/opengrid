//! The JSON forms of the types, through the one codec of `opengrid-json`
//! (issue #41) — the forms the browser and the server read and write alike.

use opengrid_json::{Error, Fields, FromJson, Json, Number, ToJson, unknown_variant};

use crate::{
    DataSourceId, DataType, DatePart, Derivation, Field, FieldName, Schema, Value, ValueError,
};

macro_rules! identifier {
    ($name:ident) => {
        impl FromJson for $name {
            /// A string that follows the identifier rule; anything else says why.
            fn from_json(json: &Json) -> Result<Self, Error> {
                let text = String::from_json(json)?;
                $name::new(text).map_err(|error| Error::new(error.to_string()))
            }
        }

        impl ToJson for $name {
            fn to_json(&self) -> Json {
                Json::from(self.as_str())
            }
        }
    };
}

identifier!(FieldName);
identifier!(DataSourceId);

const TYPES: [&str; 7] = [
    "bool",
    "int64",
    "float64",
    "decimal",
    "utf8",
    "date",
    "timestamp",
];

impl FromJson for DataType {
    /// `"int64"`, or `{"decimal": {"precision": 12, "scale": 2}}` — read
    /// through [`DataType::decimal`], so an impossible decimal cannot enter.
    fn from_json(json: &Json) -> Result<Self, Error> {
        match json {
            Json::String(name) => match name.as_str() {
                "bool" => Ok(DataType::Bool),
                "int64" => Ok(DataType::Int64),
                "float64" => Ok(DataType::Float64),
                "utf8" => Ok(DataType::Utf8),
                "date" => Ok(DataType::Date),
                "timestamp" => Ok(DataType::Timestamp),
                // The one type with parameters is an object, never a bare name.
                "decimal" => Err(Error::new(
                    "invalid type: unit variant, expected struct variant",
                )),
                other => Err(unknown_variant(other, &TYPES)),
            },
            Json::Object(object) => {
                let mut members = object.iter();
                let (Some((name, body)), None) = (members.next(), members.next()) else {
                    return Err(json.invalid_type("a type name or one {\"decimal\": …}"));
                };
                if name != "decimal" {
                    return Err(match TYPES.contains(&name.as_str()) {
                        true => Error::new("invalid type: map, expected unit variant"),
                        false => unknown_variant(name, &TYPES),
                    });
                }
                let fields = Fields::of(
                    body,
                    "struct variant DataTypeRepr::Decimal",
                    &["precision", "scale"],
                )?;
                DataType::decimal(fields.read("precision")?, fields.read("scale")?)
                    .map_err(|error| Error::new(error.to_string()))
            }
            other => Err(other.invalid_type("enum DataTypeRepr")),
        }
    }
}

impl ToJson for DataType {
    fn to_json(&self) -> Json {
        match *self {
            DataType::Bool => Json::from("bool"),
            DataType::Int64 => Json::from("int64"),
            DataType::Float64 => Json::from("float64"),
            DataType::Decimal { precision, scale } => Json::object([(
                "decimal",
                Json::object([("precision", precision), ("scale", scale)]),
            )]),
            DataType::Utf8 => Json::from("utf8"),
            DataType::Date => Json::from("date"),
            DataType::Timestamp => Json::from("timestamp"),
        }
    }
}

impl FromJson for DatePart {
    fn from_json(json: &Json) -> Result<Self, Error> {
        match String::from_json(json)?.as_str() {
            "year" => Ok(DatePart::Year),
            "month" => Ok(DatePart::Month),
            other => Err(unknown_variant(other, &["year", "month"])),
        }
    }
}

impl ToJson for DatePart {
    fn to_json(&self) -> Json {
        Json::from(self.as_str())
    }
}

impl FromJson for Derivation {
    fn from_json(json: &Json) -> Result<Self, Error> {
        let fields = Fields::of(json, "struct Derivation", &["part", "field"])?;
        Ok(Derivation {
            part: fields.read("part")?,
            field: fields.read("field")?,
        })
    }
}

impl ToJson for Derivation {
    fn to_json(&self) -> Json {
        Json::object([
            ("part", self.part.to_json()),
            ("field", self.field.to_json()),
        ])
    }
}

impl FromJson for Field {
    /// `{ "name", "type", "nullable", "from" }`; a missing `nullable` is
    /// `false` — a column is required unless the schema says otherwise.
    fn from_json(json: &Json) -> Result<Self, Error> {
        let fields = Fields::of(json, "struct Field", &["name", "type", "nullable", "from"])?;
        Ok(Field {
            name: fields.read("name")?,
            data_type: fields.read("type")?,
            nullable: fields.read_or_default("nullable")?,
            from: fields.read_optional("from")?,
        })
    }
}

impl ToJson for Field {
    fn to_json(&self) -> Json {
        let mut members = vec![
            ("name", self.name.to_json()),
            ("type", self.data_type.to_json()),
            ("nullable", Json::from(self.nullable)),
        ];
        if let Some(from) = &self.from {
            members.push(("from", from.to_json()));
        }
        Json::object(members)
    }
}

impl FromJson for Schema {
    /// `{ "fields": [ … ] }`.
    fn from_json(json: &Json) -> Result<Self, Error> {
        let fields = Fields::of(json, "struct Schema", &["fields"])?;
        Ok(Schema::new(fields.read_or_default("fields")?))
    }
}

impl ToJson for Schema {
    fn to_json(&self) -> Json {
        Json::object([("fields", self.fields().to_json())])
    }
}

impl ToJson for Value {
    /// The wire notation (E13): a decimal, a date and a timestamp as their
    /// strings, a non-finite float as `"NaN"`, `"Infinity"` or `"-Infinity"` —
    /// never `null`, which is a different value (S1 vs S7).
    fn to_json(&self) -> Json {
        match self {
            Value::Null => Json::Null,
            Value::Bool(flag) => Json::Bool(*flag),
            Value::Int64(value) => Json::from(*value),
            Value::Float64(value) => match crate::value::non_finite_name(*value) {
                Some(name) => Json::from(name),
                None => Json::from(*value),
            },
            Value::Decimal(value) => Json::from(value.to_string()),
            Value::Utf8(text) => Json::from(text.as_str()),
            Value::Date(date) => Json::from(date.to_string()),
            Value::Timestamp(timestamp) => Json::from(timestamp.to_string()),
        }
    }
}

impl Value {
    /// Reads a JSON scalar as a value of the column type `data_type`.
    ///
    /// JSON alone cannot tell a decimal string from a `Utf8` string, so the
    /// caller supplies the type (the column-oriented wire format, E6). `null` is
    /// accepted for every type; an array or an object for none.
    pub fn from_json_typed(json: &Json, data_type: &DataType) -> Result<Self, ValueError> {
        let raw = match json {
            Json::Null => crate::value::Raw::Null,
            Json::Bool(flag) => crate::value::Raw::Bool(*flag),
            Json::Number(Number::PosInt(value)) => crate::value::Raw::UInt(*value),
            Json::Number(Number::NegInt(value)) => crate::value::Raw::Int(*value),
            Json::Number(Number::Float(value)) => crate::value::Raw::Float(*value),
            Json::String(text) => crate::value::Raw::Str(text.clone()),
            Json::Array(_) => {
                return Err(ValueError::TypeMismatch {
                    expected: *data_type,
                    found: "array",
                });
            }
            Json::Object(_) => {
                return Err(ValueError::TypeMismatch {
                    expected: *data_type,
                    found: "object",
                });
            }
        };
        crate::value::coerce(raw, data_type)
    }
}
