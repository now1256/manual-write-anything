//! Backend operations from an OpenAPI document, matched by shape.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::urlpat::{match_score, normalize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Operation {
    pub method: String,
    /// Path with parameters as `{}`.
    pub path: String,
    /// Path as written in the document (`/datasets/{dataset_id}`).
    pub raw_path: String,
    pub operation_id: Option<String>,
    pub summary: Option<String>,
    pub tags: Vec<String>,
}

pub fn operations(doc: &Value) -> Vec<Operation> {
    let mut out = Vec::new();
    let Some(paths) = doc["paths"].as_object() else {
        return out;
    };
    for (raw_path, item) in paths {
        let Some(item) = item.as_object() else {
            continue;
        };
        let Some(path) = normalize(raw_path) else {
            continue;
        };
        for (method, op) in item {
            let m = method.to_uppercase();
            if !matches!(
                m.as_str(),
                "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS"
            ) {
                continue;
            }
            out.push(Operation {
                method: m,
                path: path.clone(),
                raw_path: raw_path.clone(),
                operation_id: op["operationId"].as_str().map(String::from),
                summary: op["summary"].as_str().map(String::from),
                tags: op["tags"]
                    .as_array()
                    .map(|t| {
                        t.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }
    }
    out.sort_by(|a, b| (&a.path, &a.method).cmp(&(&b.path, &b.method)));
    out
}

/// Best operation for `method path`: same method, same shape, most literal segments in common.
pub fn find<'a>(ops: &'a [Operation], method: &str, path: &str) -> Option<&'a Operation> {
    ops.iter()
        .filter(|o| o.method == method)
        .filter_map(|o| match_score(&o.path, path).map(|s| (s, o)))
        .max_by_key(|(s, _)| *s)
        .map(|(_, o)| o)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_and_matches_by_shape() {
        let doc = json!({"paths": {
            "/datasets/{dataset_id}": {"get": {"operationId": "get_ds", "tags": ["ds"]}, "delete": {}},
            "/datasets/summary": {"get": {"summary": "요약"}},
            "/datasets": {"post": {}, "parameters": []}
        }});
        let ops = operations(&doc);
        assert_eq!(ops.len(), 4);
        assert_eq!(
            find(&ops, "GET", "/datasets/{}")
                .unwrap()
                .operation_id
                .as_deref(),
            Some("get_ds")
        );
        assert_eq!(
            find(&ops, "GET", "/datasets/summary").unwrap().raw_path,
            "/datasets/summary",
            "literal beats param"
        );
        assert_eq!(
            find(&ops, "DELETE", "/datasets/{}").unwrap().raw_path,
            "/datasets/{dataset_id}"
        );
        assert!(find(&ops, "PUT", "/datasets/{}").is_none());
        assert!(find(&ops, "POST", "/datasets").is_some());
    }
}
