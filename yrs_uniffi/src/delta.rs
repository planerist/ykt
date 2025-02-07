use crate::attrs::{from_yattrs_opt, from_yvalue, YAttributes, YValue};
use yrs::types::Delta;
use yrs::Any;

#[derive(uniffi::Enum)]
pub enum YDelta {
    YInsert(YValue, Option<YAttributes>),
    YDelete(u32),
    YRetain(u32, Option<YAttributes>),
}

pub fn y_into_delta(d: &YDelta) -> Delta<Any> {
    match d {
        YDelta::YInsert(v, attrs) => Delta::Inserted(from_yvalue(v), from_yattrs_opt(attrs)),
        YDelta::YDelete(len) => Delta::Deleted(*len),
        YDelta::YRetain(len, attrs) => Delta::Retain(*len, from_yattrs_opt(attrs)),
    }
}
