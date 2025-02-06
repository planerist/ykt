use crate::tools::Error;
use yrs::types::Attrs;
use yrs::Any;

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
