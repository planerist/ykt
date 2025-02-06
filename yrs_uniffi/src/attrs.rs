use crate::tools::Error;
use yrs::types::Attrs;
use yrs::Any;

// #[uniffi::export]
// fn ytrue() -> YBoolAttr {
//     YBoolAttr::new(true)
// }
//
// #[uniffi::export]
// fn yfalse() -> YBoolAttr {
//     YBoolAttr::new(false)
// }
//
// #[uniffi::export]
// pub trait YAttr: Send + Sync {
//
// }
//
// impl YAttr for String {
// }

//
// #[derive(uniffi::Object)]
// pub struct YMapAttr {}
//
// #[derive(uniffi::Object)]
// pub struct YBoolAttr {
//     value: bool
// }

// impl YBoolAttr {
//     pub fn new(v: bool) -> Self {
//         YBoolAttr {
//             value: v
//         }
//     }
// }
//
// #[uniffi::export]
// impl YAttr for YMapAttr {
// }

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
