use crate::tools::Error;
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;
use yrs::types::Attrs;
use yrs::Any;

pub type YAttributes = HashMap<String, YValue>;

#[derive(uniffi::Enum)]
pub enum YValue {
    Null,
    Bool(bool),
    Number(f64),
    BigInt(i64),
    String(String),
    Buffer(Vec<u8>),
    Array(Vec<YValue>),
    AttrMap(YAttributes),
}

pub fn into_yattrs(attrs: Attrs) -> YAttributes {
    let mut result: YAttributes = HashMap::new();
    for (k, v) in attrs {
        result.insert(k.to_string(), into_yvalue(&v));
    }

    result
}

fn into_yattrs2(attrs: &HashMap<String, Any>) -> YAttributes {
    let mut result: YAttributes = HashMap::new();
    for (k, v) in attrs {
        result.insert(k.to_string(), into_yvalue(v));
    }

    result
}

pub fn into_yvalue(v: &Any) -> YValue {
    match v {
        Any::Null => YValue::Null,
        Any::Undefined => YValue::Null,
        Any::Bool(v) => YValue::Bool(*v),
        Any::Number(v) => YValue::Number(*v),
        Any::BigInt(v) => YValue::BigInt(*v),
        Any::String(v) => YValue::String(v.to_string()),
        Any::Buffer(v) => YValue::Buffer(v.to_vec()),
        Any::Array(v) => {
            YValue::Array(v.into_iter().map(|x| into_yvalue(x)).collect())
        }
        Any::Map(v) => {
            let v = v.clone();
            YValue::AttrMap(into_yattrs2(v.deref()))
        }
    }
}

pub fn from_yvalue(v: &YValue) -> Any {
    match v {
        YValue::Null => Any::Null,
        YValue::Bool(v) => Any::Bool(*v),
        YValue::Number(v) => Any::Number(*v),
        YValue::BigInt(v) => Any::BigInt(*v),
        YValue::String(v) => Any::String(Arc::from(v.to_string())),
        YValue::Buffer(v) => Any::Buffer(Arc::from(v.as_slice())),
        YValue::Array(v) => Any::Array(v.iter().map(|x| from_yvalue(x)).collect()),
        YValue::AttrMap(v) => {
            let mut res : HashMap<String, Any> = HashMap::new();
            for (k, v) in v {
                res.insert(k.to_string(), from_yvalue(v));
            }
            Any::Map(Arc::from(res))
        }
    }
}

pub fn from_yattrs_opt(attr: &Option<YAttributes>) -> Option<Box<Attrs>> {
    if let Some(v) = attr {
        Some(Box::from(from_yattrs(&v)))
    } else {
        None
    }
}

pub fn from_yattrs(attrs: &YAttributes) -> Attrs {
    let mut res : HashMap<Arc<str>, Any> = HashMap::new();
    for (k, v) in attrs {
        res.insert(Arc::from(k.as_str()), from_yvalue(v));
    }

    res
}


pub fn parse_attrs(attributes: Option<String>) -> crate::tools::Result<Option<Attrs>> {
    match attributes {
        None => Ok(None),
        Some(str) => {
            let parsed = Any::from_json(&str);
            match parsed {
                Err(_) => Err(Error::InvalidFmt),
                Ok(any) => {
                    let attrs = map_attrs(any);
                    if let Some(_) = attrs {
                        Ok(attrs)
                    } else {
                        Err(Error::InvalidFmt)
                    }
                }
            }
        }
    }
}

fn map_attrs(attrs: Any) -> Option<Attrs> {
    if let Any::Map(attrs) = attrs {
        let attrs = attrs
            .iter()
            .map(|(k, v)| (k.as_str().into(), v.clone()))
            .collect();
        Some(attrs)
    } else {
        None
    }
}
