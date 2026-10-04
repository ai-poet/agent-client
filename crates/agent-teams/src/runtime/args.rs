//! Reading tool arguments from the model's JSON.
//!
//! The model's arguments arrive as one JSON object. These helpers check the
//! types the schemas promise and name the offending field, so a malformed
//! call comes back as an error the model can correct.

use serde_json::{Map, Value};

use super::OpResult;

/// One call's arguments.
pub struct Args<'a> {
    tool: &'a str,
    map: Map<String, Value>,
}

impl<'a> Args<'a> {
    pub fn new(tool: &'a str, input: &Value) -> OpResult<Self> {
        match input {
            Value::Object(map) => Ok(Self {
                tool,
                map: map.clone(),
            }),
            Value::Null => Ok(Self {
                tool,
                map: Map::new(),
            }),
            _ => Err(format!("{tool}: arguments must be a JSON object")),
        }
    }

    pub fn map(&self) -> &Map<String, Value> {
        &self.map
    }

    fn present(&self, key: &str) -> Option<&Value> {
        self.map.get(key).filter(|value| !value.is_null())
    }

    pub fn has(&self, key: &str) -> bool {
        self.present(key).is_some()
    }

    pub fn raw(&self, key: &str) -> Option<&Value> {
        self.present(key)
    }

    /// An optional string, kept as given (not trimmed).
    pub fn opt_str(&self, key: &str) -> OpResult<Option<String>> {
        match self.present(key) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(format!("{}: \"{key}\" must be a string", self.tool)),
        }
    }

    pub fn req_str(&self, key: &str) -> OpResult<String> {
        self.opt_str(key)?
            .ok_or_else(|| format!("{}: \"{key}\" is required", self.tool))
    }

    pub fn opt_bool(&self, key: &str) -> OpResult<Option<bool>> {
        match self.present(key) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(*flag)),
            Some(_) => Err(format!("{}: \"{key}\" must be a boolean", self.tool)),
        }
    }

    /// An optional positive whole number; a float with no fraction is
    /// accepted, as JSON numbers from models often are.
    pub fn opt_u32(&self, key: &str) -> OpResult<Option<u32>> {
        match self.present(key) {
            None => Ok(None),
            Some(Value::Number(number)) => {
                let value = number
                    .as_u64()
                    .or_else(|| {
                        number
                            .as_f64()
                            .filter(|value| value.fract() == 0.0 && *value >= 0.0)
                            .map(|value| value as u64)
                    })
                    .and_then(|value| u32::try_from(value).ok());
                value.map(Some).ok_or_else(|| {
                    format!("{}: \"{key}\" must be a non-negative integer", self.tool)
                })
            }
            Some(_) => Err(format!("{}: \"{key}\" must be a number", self.tool)),
        }
    }

    /// An optional list of strings.
    pub fn opt_strings(&self, key: &str) -> OpResult<Option<Vec<String>>> {
        match self.present(key) {
            None => Ok(None),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_str().map(str::to_owned).ok_or_else(|| {
                        format!("{}: \"{key}\" must be a list of strings", self.tool)
                    })
                })
                .collect::<OpResult<Vec<_>>>()
                .map(Some),
            Some(_) => Err(format!(
                "{}: \"{key}\" must be a list of strings",
                self.tool
            )),
        }
    }

    /// An optional list of objects.
    pub fn opt_objects(&self, key: &str) -> OpResult<Option<Vec<Map<String, Value>>>> {
        match self.present(key) {
            None => Ok(None),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_object().cloned().ok_or_else(|| {
                        format!("{}: \"{key}\" must be a list of objects", self.tool)
                    })
                })
                .collect::<OpResult<Vec<_>>>()
                .map(Some),
            Some(_) => Err(format!(
                "{}: \"{key}\" must be a list of objects",
                self.tool
            )),
        }
    }

    /// One of a fixed set of strings.
    pub fn opt_enum(&self, key: &str, allowed: &[&str]) -> OpResult<Option<String>> {
        let Some(value) = self.opt_str(key)? else {
            return Ok(None);
        };
        if allowed.contains(&value.as_str()) {
            Ok(Some(value))
        } else {
            Err(format!(
                "{}: \"{key}\" must be one of {}",
                self.tool,
                allowed.join(", ")
            ))
        }
    }
}

/// A sub-object (a plan member, an edit operation) read with the same rules.
pub fn sub<'a>(tool: &'a str, map: &Map<String, Value>) -> Args<'a> {
    Args {
        tool,
        map: map.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_are_checked_and_null_reads_as_absent() {
        let input = serde_json::json!({
            "name": "x", "flag": true, "n": 3.0, "list": ["a"], "none": null, "bad": 4
        });
        let args = Args::new("t", &input).unwrap();
        assert_eq!(args.req_str("name").unwrap(), "x");
        assert_eq!(args.opt_bool("flag").unwrap(), Some(true));
        assert_eq!(args.opt_u32("n").unwrap(), Some(3));
        assert_eq!(
            args.opt_strings("list").unwrap(),
            Some(vec!["a".to_owned()])
        );
        assert_eq!(args.opt_str("none").unwrap(), None);
        assert_eq!(
            args.opt_str("bad").unwrap_err(),
            "t: \"bad\" must be a string"
        );
        assert_eq!(
            args.req_str("missing").unwrap_err(),
            "t: \"missing\" is required"
        );
        assert!(args.opt_enum("name", &["y"]).is_err());
    }
}
