//! The one JSON codec of opengrid (issue #41).
//!
//! The browser modules used to carry `serde` and `serde_json`, and the server
//! read the same query, schema and result JSON through them. This crate
//! replaces both, for both sides: one reader and one writer, so the browser
//! and the server cannot read the same bytes two ways (decision E35).
//!
//! It is deliberately small — a document tree ([`Json`]), a strict reader
//! ([`Json::parse`], RFC 8259), a compact writer ([`Json::to_string`]) and two
//! traits ([`FromJson`], [`ToJson`]) the opengrid types implement by hand.
//! [`Fields`] carries the one rule every opengrid object follows: an unknown
//! key is an error, never ignored.
//!
//! **What it writes is what `serde_json` wrote**, with one exception: a float
//! is written with the shortest digits that read back to the same number, as
//! `serde_json` does, but where two such spellings exist this crate takes the
//! one closest to the value (Rust's own choice) and `serde_json` may take the
//! other. Both read back to the identical `f64`.
//!
//! **Object keys keep their order.** A writer decides the order it writes in;
//! a reader never depends on it.

mod parse;
mod write;

use std::fmt;

pub use write::write_f64;

/// A JSON document.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Json {
    #[default]
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Json>),
    Object(Object),
}

/// A JSON number, kept as the integer it was when it was one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Number {
    /// A non-negative integer.
    PosInt(u64),
    /// A negative integer.
    NegInt(i64),
    /// Anything with a fraction or an exponent, or an integer past 64 bits.
    Float(f64),
}

impl Number {
    /// The number as an `i64`, if it is an integer that fits.
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Number::PosInt(value) => i64::try_from(value).ok(),
            Number::NegInt(value) => Some(value),
            Number::Float(_) => None,
        }
    }

    /// The number as a `u64`, if it is a non-negative integer.
    pub fn as_u64(&self) -> Option<u64> {
        match *self {
            Number::PosInt(value) => Some(value),
            _ => None,
        }
    }

    /// The number as an `f64` — always, an integer converted.
    pub fn as_f64(&self) -> f64 {
        match *self {
            Number::PosInt(value) => value as f64,
            Number::NegInt(value) => value as f64,
            Number::Float(value) => value,
        }
    }

    /// Whether the number was written as a float.
    pub fn is_float(&self) -> bool {
        matches!(self, Number::Float(_))
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Number::PosInt(value) => write!(f, "{value}"),
            Number::NegInt(value) => write!(f, "{value}"),
            Number::Float(value) => f.write_str(&write_f64(value)),
        }
    }
}

/// An object: its members in the order they were read or inserted.
///
/// A key may occur twice in what was read; [`Object::get`] answers the last
/// one, as a JSON reader customarily does, and [`Fields`] refuses the object.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Object(Vec<(String, Json)>);

impl Object {
    /// An empty object.
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// The value of `key`, the last one if the key occurs twice.
    pub fn get(&self, key: &str) -> Option<&Json> {
        self.0
            .iter()
            .rev()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    /// Sets `key`: replaces its value where it is, or appends it.
    pub fn insert(&mut self, key: impl Into<String>, value: impl Into<Json>) {
        let key = key.into();
        let value = value.into();
        match self.0.iter_mut().find(|(name, _)| *name == key) {
            Some(entry) => entry.1 = value,
            None => self.0.push((key, value)),
        }
    }

    /// Removes `key` and answers its value.
    pub fn remove(&mut self, key: &str) -> Option<Json> {
        let at = self.0.iter().rposition(|(name, _)| name == key)?;
        Some(self.0.remove(at).1)
    }

    /// Whether `key` is there.
    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// The members, in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Json)> {
        self.0.iter().map(|(key, value)| (key.as_str(), value))
    }

    /// The keys, in order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|(key, _)| key.as_str())
    }

    /// Number of members.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the object has no members.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn push(&mut self, key: String, value: Json) {
        self.0.push((key, value));
    }
}

impl<K: Into<String>, V: Into<Json>> FromIterator<(K, V)> for Object {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(members: I) -> Self {
        let mut object = Object::new();
        for (key, value) in members {
            object.insert(key, value);
        }
        object
    }
}

/// `null`, for [`Json`]'s index operators to point at when a key is missing.
static NULL: Json = Json::Null;

impl Json {
    /// Reads a document (RFC 8259), strictly: nothing after the value but
    /// whitespace, no comments, no trailing commas, no lone surrogates, at
    /// most 128 levels of nesting.
    pub fn parse(text: &str) -> Result<Json, Error> {
        parse::parse(text)
    }

    /// An object of `members`, in that order.
    pub fn object<K: Into<String>, V: Into<Json>>(
        members: impl IntoIterator<Item = (K, V)>,
    ) -> Json {
        Json::Object(members.into_iter().collect())
    }

    /// Whether this is `null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    /// The string, if this is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(text) => Some(text),
            _ => None,
        }
    }

    /// The boolean, if this is one.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(flag) => Some(*flag),
            _ => None,
        }
    }

    /// The number, if this is one.
    pub fn as_number(&self) -> Option<&Number> {
        match self {
            Json::Number(number) => Some(number),
            _ => None,
        }
    }

    /// The number as an `i64`, if this is an integer that fits.
    pub fn as_i64(&self) -> Option<i64> {
        self.as_number().and_then(Number::as_i64)
    }

    /// The number as a `u64`, if this is a non-negative integer.
    pub fn as_u64(&self) -> Option<u64> {
        self.as_number().and_then(Number::as_u64)
    }

    /// The number as an `f64`, if this is a number.
    pub fn as_f64(&self) -> Option<f64> {
        self.as_number().map(Number::as_f64)
    }

    /// The elements, if this is an array.
    pub fn as_array(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The elements, mutably, if this is an array.
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Json>> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    /// The members, if this is an object.
    pub fn as_object(&self) -> Option<&Object> {
        match self {
            Json::Object(object) => Some(object),
            _ => None,
        }
    }

    /// The members, mutably, if this is an object.
    pub fn as_object_mut(&mut self) -> Option<&mut Object> {
        match self {
            Json::Object(object) => Some(object),
            _ => None,
        }
    }

    /// The value of `key`, if this is an object that has it.
    pub fn get(&self, key: &str) -> Option<&Json> {
        self.as_object().and_then(|object| object.get(key))
    }

    /// How this value is named in an error: `null`, `boolean `true``,
    /// `integer `5``, `floating point `1.5``, `string "x"`, `sequence`, `map` —
    /// the words `serde_json` used, so a message reads as it did.
    pub fn unexpected(&self) -> String {
        match self {
            Json::Null => "null".to_owned(),
            Json::Bool(flag) => format!("boolean `{flag}`"),
            Json::Number(Number::Float(value)) => {
                format!("floating point `{}`", write_f64(*value))
            }
            Json::Number(number) => format!("integer `{number}`"),
            Json::String(text) => format!("string {text:?}"),
            Json::Array(_) => "sequence".to_owned(),
            Json::Object(_) => "map".to_owned(),
        }
    }

    /// An error saying this value is not `expected`.
    pub fn invalid_type(&self, expected: &str) -> Error {
        Error::new(format!(
            "invalid type: {}, expected {expected}",
            self.unexpected()
        ))
    }
}

impl fmt::Display for Json {
    /// The compact form: no whitespace, keys in their order.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        write::write(self, &mut out);
        f.write_str(&out)
    }
}

impl std::ops::Index<&str> for Json {
    type Output = Json;

    /// The value of `key`, or `null` when this is no object or has no such key.
    fn index(&self, key: &str) -> &Json {
        self.get(key).unwrap_or(&NULL)
    }
}

impl std::ops::Index<usize> for Json {
    type Output = Json;

    /// The element at `index`, or `null` when this is no array or is shorter.
    fn index(&self, index: usize) -> &Json {
        self.as_array()
            .and_then(|items| items.get(index))
            .unwrap_or(&NULL)
    }
}

impl From<bool> for Json {
    fn from(flag: bool) -> Self {
        Json::Bool(flag)
    }
}

impl From<&str> for Json {
    fn from(text: &str) -> Self {
        Json::String(text.to_owned())
    }
}

impl From<String> for Json {
    fn from(text: String) -> Self {
        Json::String(text)
    }
}

impl From<i64> for Json {
    fn from(value: i64) -> Self {
        Json::Number(if value < 0 {
            Number::NegInt(value)
        } else {
            Number::PosInt(value as u64)
        })
    }
}

impl From<i32> for Json {
    fn from(value: i32) -> Self {
        Json::from(i64::from(value))
    }
}

impl From<u64> for Json {
    fn from(value: u64) -> Self {
        Json::Number(Number::PosInt(value))
    }
}

impl From<u32> for Json {
    fn from(value: u32) -> Self {
        Json::from(u64::from(value))
    }
}

impl From<u16> for Json {
    fn from(value: u16) -> Self {
        Json::from(u64::from(value))
    }
}

impl From<u8> for Json {
    fn from(value: u8) -> Self {
        Json::from(u64::from(value))
    }
}

impl From<usize> for Json {
    fn from(value: usize) -> Self {
        Json::from(value as u64)
    }
}

impl From<f64> for Json {
    /// A finite float; NaN and the infinities have no JSON spelling and
    /// become `null`, as they did with `serde_json`.
    fn from(value: f64) -> Self {
        if value.is_finite() {
            Json::Number(Number::Float(value))
        } else {
            Json::Null
        }
    }
}

impl From<Vec<Json>> for Json {
    fn from(items: Vec<Json>) -> Self {
        Json::Array(items)
    }
}

impl From<Object> for Json {
    fn from(object: Object) -> Self {
        Json::Object(object)
    }
}

impl<T: Into<Json>> From<Option<T>> for Json {
    fn from(value: Option<T>) -> Self {
        value.map_or(Json::Null, Into::into)
    }
}

/// What went wrong reading JSON: bytes that are not JSON, or JSON that is not
/// the value a reader expected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    /// An error with this sentence.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// The sentence.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// A value that can be read from JSON.
pub trait FromJson: Sized {
    /// Reads the value, or says why the JSON is not one.
    fn from_json(json: &Json) -> Result<Self, Error>;
}

/// A value that can be written as JSON.
pub trait ToJson {
    /// The value as a document.
    fn to_json(&self) -> Json;
}

/// Reads a `T` from JSON text.
pub fn from_str<T: FromJson>(text: &str) -> Result<T, Error> {
    T::from_json(&Json::parse(text)?)
}

/// Writes a `T` as compact JSON text.
pub fn to_string<T: ToJson + ?Sized>(value: &T) -> String {
    value.to_json().to_string()
}

/// The members of an object that stands for a struct: each key known, none
/// twice. The rule every opengrid object follows — a typo in a request is an
/// error, never a field silently ignored.
pub struct Fields<'a> {
    object: &'a Object,
}

impl<'a> Fields<'a> {
    /// The object's members, refusing anything but an object (`what` names
    /// it in the error), a key outside `known`, and a key that occurs twice.
    pub fn of(json: &'a Json, what: &str, known: &[&str]) -> Result<Self, Error> {
        let Json::Object(object) = json else {
            return Err(json.invalid_type(what));
        };
        for (index, (key, _)) in object.0.iter().enumerate() {
            if !known.contains(&key.as_str()) {
                return Err(Error::new(unknown_field(key, known)));
            }
            if object.0[..index].iter().any(|(earlier, _)| earlier == key) {
                return Err(Error::new(format!("duplicate field `{key}`")));
            }
        }
        Ok(Self { object })
    }

    /// The value of `key`, if present — `null` included.
    pub fn get(&self, key: &str) -> Option<&'a Json> {
        self.object.get(key)
    }

    /// The value of `key` unless it is missing or `null` — how an optional
    /// field reads.
    pub fn optional(&self, key: &str) -> Option<&'a Json> {
        self.get(key).filter(|value| !value.is_null())
    }

    /// The value of `key`, or the error that it is missing.
    pub fn required(&self, key: &str) -> Result<&'a Json, Error> {
        self.get(key)
            .ok_or_else(|| Error::new(format!("missing field `{key}`")))
    }

    /// `key` read as a `T`, or the error that it is missing.
    pub fn read<T: FromJson>(&self, key: &str) -> Result<T, Error> {
        T::from_json(self.required(key)?)
    }

    /// `key` read as a `T` when present and not `null`.
    pub fn read_optional<T: FromJson>(&self, key: &str) -> Result<Option<T>, Error> {
        self.optional(key).map(T::from_json).transpose()
    }

    /// `key` read as a `T`, or `T::default()` when missing. `null` is read as
    /// a `T` like any other value.
    pub fn read_or_default<T: FromJson + Default>(&self, key: &str) -> Result<T, Error> {
        self.get(key)
            .map(T::from_json)
            .transpose()
            .map(Option::unwrap_or_default)
    }
}

/// The sentence for a key a struct does not have.
fn unknown_field(key: &str, known: &[&str]) -> String {
    match known {
        [] => format!("unknown field `{key}`, there are no fields"),
        [only] => format!("unknown field `{key}`, expected `{only}`"),
        [first, rest @ ..] => {
            let mut message = format!("unknown field `{key}`, expected one of `{first}`");
            for name in rest {
                message.push_str(&format!(", `{name}`"));
            }
            message
        }
    }
}

/// The sentence for a string that is none of `variants`.
pub fn unknown_variant(value: &str, variants: &[&str]) -> Error {
    let mut message = format!("unknown variant `{value}`, expected ");
    match variants {
        [only] => message.push_str(&format!("`{only}`")),
        [first, rest @ ..] => {
            message.push_str(&format!("one of `{first}`"));
            for name in rest {
                message.push_str(&format!(", `{name}`"));
            }
        }
        [] => message.push_str("nothing"),
    }
    Error::new(message)
}

// ---------------------------------------------------------------------------
// The common types
// ---------------------------------------------------------------------------

impl FromJson for Json {
    fn from_json(json: &Json) -> Result<Self, Error> {
        Ok(json.clone())
    }
}

impl ToJson for Json {
    fn to_json(&self) -> Json {
        self.clone()
    }
}

impl FromJson for String {
    fn from_json(json: &Json) -> Result<Self, Error> {
        json.as_str()
            .map(str::to_owned)
            .ok_or_else(|| json.invalid_type("a string"))
    }
}

impl ToJson for String {
    fn to_json(&self) -> Json {
        Json::String(self.clone())
    }
}

impl ToJson for str {
    fn to_json(&self) -> Json {
        Json::String(self.to_owned())
    }
}

impl FromJson for bool {
    fn from_json(json: &Json) -> Result<Self, Error> {
        json.as_bool().ok_or_else(|| json.invalid_type("a boolean"))
    }
}

impl ToJson for bool {
    fn to_json(&self) -> Json {
        Json::Bool(*self)
    }
}

impl FromJson for f64 {
    fn from_json(json: &Json) -> Result<Self, Error> {
        json.as_f64().ok_or_else(|| json.invalid_type("f64"))
    }
}

impl ToJson for f64 {
    fn to_json(&self) -> Json {
        Json::from(*self)
    }
}

macro_rules! integer {
    ($($name:ty),*) => {$(
        impl FromJson for $name {
            fn from_json(json: &Json) -> Result<Self, Error> {
                let Some(number) = json.as_number() else {
                    return Err(json.invalid_type(stringify!($name)));
                };
                let value = match *number {
                    Number::PosInt(value) => <$name>::try_from(value).ok(),
                    Number::NegInt(value) => <$name>::try_from(value).ok(),
                    Number::Float(_) => return Err(json.invalid_type(stringify!($name))),
                };
                value.ok_or_else(|| {
                    Error::new(format!(
                        "invalid value: integer `{number}`, expected {}",
                        stringify!($name)
                    ))
                })
            }
        }

        impl ToJson for $name {
            fn to_json(&self) -> Json {
                #[allow(irrefutable_let_patterns, clippy::cast_lossless)]
                if let Ok(value) = u64::try_from(*self) {
                    Json::Number(Number::PosInt(value))
                } else {
                    Json::Number(Number::NegInt(*self as i64))
                }
            }
        }
    )*};
}

integer!(u8, u16, u32, u64, usize, i32, i64);

impl<T: FromJson> FromJson for Vec<T> {
    fn from_json(json: &Json) -> Result<Self, Error> {
        json.as_array()
            .ok_or_else(|| json.invalid_type("a sequence"))?
            .iter()
            .map(T::from_json)
            .collect()
    }
}

impl<T: ToJson> ToJson for [T] {
    fn to_json(&self) -> Json {
        Json::Array(self.iter().map(ToJson::to_json).collect())
    }
}

impl<T: ToJson> ToJson for Vec<T> {
    fn to_json(&self) -> Json {
        self.as_slice().to_json()
    }
}

impl<T: FromJson> FromJson for Option<T> {
    fn from_json(json: &Json) -> Result<Self, Error> {
        if json.is_null() {
            Ok(None)
        } else {
            T::from_json(json).map(Some)
        }
    }
}

impl<T: ToJson> ToJson for Option<T> {
    fn to_json(&self) -> Json {
        self.as_ref().map_or(Json::Null, ToJson::to_json)
    }
}

impl<T: FromJson> FromJson for Box<T> {
    fn from_json(json: &Json) -> Result<Self, Error> {
        T::from_json(json).map(Box::new)
    }
}

impl<T: ToJson + ?Sized> ToJson for Box<T> {
    fn to_json(&self) -> Json {
        (**self).to_json()
    }
}

impl<T: ToJson + ?Sized> ToJson for &T {
    fn to_json(&self) -> Json {
        (**self).to_json()
    }
}

#[cfg(test)]
mod tests;

/// A document written as JSON, the way `serde_json::json!` wrote it:
/// `json!({ "key": value, "list": [1, null, { "a": true }] })`. A value that
/// is not a literal is any expression whose type implements [`ToJson`]. Keys
/// are string literals and keep the order they are written in.
#[macro_export]
macro_rules! json {
    (null) => {
        $crate::Json::Null
    };
    ([ $($tt:tt)* ]) => {
        $crate::Json::Array($crate::__json_array!([] $($tt)*))
    };
    ({ $($tt:tt)* }) => {{
        let mut object = $crate::Object::new();
        $crate::__json_object!(object $($tt)*);
        $crate::Json::Object(object)
    }};
    ($other:expr) => {
        $crate::ToJson::to_json(&$other)
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __json_array {
    ([$($done:expr,)*]) => {
        vec![$($done,)*]
    };
    ([$($done:expr,)*] null $(, $($rest:tt)*)?) => {
        $crate::__json_array!([$($done,)* $crate::Json::Null,] $($($rest)*)?)
    };
    ([$($done:expr,)*] [$($inner:tt)*] $(, $($rest:tt)*)?) => {
        $crate::__json_array!([$($done,)* $crate::json!([$($inner)*]),] $($($rest)*)?)
    };
    ([$($done:expr,)*] {$($inner:tt)*} $(, $($rest:tt)*)?) => {
        $crate::__json_array!([$($done,)* $crate::json!({$($inner)*}),] $($($rest)*)?)
    };
    ([$($done:expr,)*] $next:expr , $($rest:tt)*) => {
        $crate::__json_array!([$($done,)* $crate::json!($next),] $($rest)*)
    };
    ([$($done:expr,)*] $last:expr) => {
        $crate::__json_array!([$($done,)* $crate::json!($last),])
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __json_object {
    ($object:ident) => {};
    ($object:ident $key:literal : null $(, $($rest:tt)*)?) => {
        $object.insert($key, $crate::Json::Null);
        $crate::__json_object!($object $($($rest)*)?);
    };
    ($object:ident $key:literal : [$($inner:tt)*] $(, $($rest:tt)*)?) => {
        $object.insert($key, $crate::json!([$($inner)*]));
        $crate::__json_object!($object $($($rest)*)?);
    };
    ($object:ident $key:literal : {$($inner:tt)*} $(, $($rest:tt)*)?) => {
        $object.insert($key, $crate::json!({$($inner)*}));
        $crate::__json_object!($object $($($rest)*)?);
    };
    ($object:ident $key:literal : $value:expr , $($rest:tt)*) => {
        $object.insert($key, $crate::json!($value));
        $crate::__json_object!($object $($rest)*);
    };
    ($object:ident $key:literal : $value:expr) => {
        $object.insert($key, $crate::json!($value));
    };
}

impl std::ops::IndexMut<&str> for Json {
    /// The value of `key`, inserted as `null` when missing.
    ///
    /// # Panics
    /// When this is not an object — a caller that writes into a document
    /// knows its shape.
    fn index_mut(&mut self, key: &str) -> &mut Json {
        let Json::Object(object) = self else {
            panic!("cannot index into {} with a key", self.unexpected());
        };
        if !object.contains_key(key) {
            object.push(key.to_owned(), Json::Null);
        }
        let at = object
            .0
            .iter()
            .rposition(|(name, _)| name == key)
            .expect("just ensured");
        &mut object.0[at].1
    }
}

impl std::ops::IndexMut<usize> for Json {
    /// The element at `index`.
    ///
    /// # Panics
    /// When this is not an array or is shorter.
    fn index_mut(&mut self, index: usize) -> &mut Json {
        let what = self.unexpected();
        match self {
            Json::Array(items) if index < items.len() => &mut items[index],
            _ => panic!("cannot index into {what} at {index}"),
        }
    }
}
