//! Turns string/template literal parts into path templates like `/api/sales/accounts/{}`.

use crate::ts::Part;

/// Builds a path template. Substitutions become `{}` segments, the query string is dropped,
/// and a trailing substitution glued to a segment (`/x${qs}`) is treated as a query suffix.
/// `resolve` may expand a substitution expression (e.g. a constant) into literal text.
/// Returns `None` when the result does not start with `/`.
pub fn template(parts: &[Part], resolve: &mut dyn FnMut(&str) -> Option<String>) -> Option<String> {
    let mut buf = String::new();
    let n = parts.len();
    for (i, part) in parts.iter().enumerate() {
        let lit = match part {
            Part::Lit(s) => Some(s.clone()),
            Part::Sub(expr) => resolve(expr),
        };
        match lit {
            Some(s) => {
                if let Some((before, _)) = s.split_once('?') {
                    buf.push_str(before);
                    break;
                }
                buf.push_str(&s);
            }
            None => {
                if buf.is_empty() {
                    return None;
                }
                let next_is_query = match parts.get(i + 1) {
                    None => true,
                    Some(Part::Lit(s)) => s.starts_with('?'),
                    Some(Part::Sub(_)) => false,
                };
                if !buf.ends_with('/') && (i + 1 == n || next_is_query) {
                    break;
                }
                buf.push_str("{}");
            }
        }
    }
    normalize(&buf)
}

/// Collapses segments containing `{}` into `{}`, drops a trailing `/`.
pub fn normalize(path: &str) -> Option<String> {
    if !path.starts_with('/') {
        return None;
    }
    let segs: Vec<&str> = path
        .split('/')
        .skip(1)
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.contains("{}") || (s.starts_with('{') && s.ends_with('}')) {
                "{}"
            } else {
                s
            }
        })
        .collect();
    Some(format!("/{}", segs.join("/")))
}

/// True if `a` and `b` have the same shape (`{}` matches any single segment).
/// Returns the number of exactly matching literal segments as a score.
pub fn match_score(a: &str, b: &str) -> Option<usize> {
    let sa: Vec<&str> = a.split('/').collect();
    let sb: Vec<&str> = b.split('/').collect();
    if sa.len() != sb.len() {
        return None;
    }
    let mut score = 0;
    for (x, y) in sa.iter().zip(&sb) {
        if x == y {
            score += usize::from(!x.is_empty());
        } else if *x != "{}" && *y != "{}" {
            return None;
        }
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(s: &str) -> Part {
        Part::Lit(s.into())
    }
    fn sub(s: &str) -> Part {
        Part::Sub(s.into())
    }
    fn t(parts: &[Part]) -> Option<String> {
        template(parts, &mut |e| {
            (e == "BASE").then(|| "/api/base".to_string())
        })
    }

    #[test]
    fn templates() {
        assert_eq!(
            t(&[lit("/api/x/"), sub("id"), lit("?db="), sub("db")]).as_deref(),
            Some("/api/x/{}")
        );
        assert_eq!(
            t(&[lit("/api/x?"), sub("params")]).as_deref(),
            Some("/api/x")
        );
        assert_eq!(
            t(&[lit("/api/x"), sub("qs")]).as_deref(),
            Some("/api/x"),
            "glued trailing sub is a query"
        );
        assert_eq!(
            t(&[lit("/api/x/"), sub("id"), lit("/test")]).as_deref(),
            Some("/api/x/{}/test")
        );
        assert_eq!(
            t(&[lit("/api/x/v-"), sub("id"), lit("/y")]).as_deref(),
            Some("/api/x/{}/y")
        );
        assert_eq!(
            t(&[sub("BASE"), lit("/jobs")]).as_deref(),
            Some("/api/base/jobs")
        );
        assert_eq!(t(&[sub("unknown"), lit("/jobs")]), None);
        assert_eq!(t(&[lit("/api/x/")]).as_deref(), Some("/api/x"));
        assert_eq!(normalize("/a/{id}/b").as_deref(), Some("/a/{}/b"));
    }

    #[test]
    fn scores_prefer_literal_segments() {
        assert_eq!(match_score("/a/{}/b", "/a/x/b"), Some(2));
        assert_eq!(match_score("/a/summary", "/a/{}"), Some(1));
        assert_eq!(match_score("/a/summary", "/a/summary"), Some(2));
        assert_eq!(match_score("/a/b", "/a/c"), None);
        assert_eq!(match_score("/a", "/a/b"), None);
    }
}
