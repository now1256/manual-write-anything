//! Blocks → Markdown with block anchors; screenshots are numbered here, not in the blocks (F4).

use crate::model::{Block, BlockKind};

fn kind_name(k: &BlockKind) -> String {
    serde_json::to_value(k)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

/// `image_prefix` is prepended to shot ids, e.g. `../shots/` → `../shots/menus/x.png`.
pub fn markdown(
    title: &str,
    blocks: &[Block],
    image_prefix: &str,
    captions: &dyn Fn(&str) -> String,
) -> String {
    let mut out = format!("# {title}\n\n");
    let mut n = 0;
    for b in blocks {
        out.push_str(&format!(
            "<!-- blk:{} kind={} facts={} -->\n",
            b.id,
            kind_name(&b.kind),
            b.facts.join(",")
        ));
        match b.kind {
            BlockKind::Image => {
                n += 1;
                let shot = b.shot.clone().unwrap_or_default();
                let caption = if b.markdown.trim().is_empty() {
                    captions(&shot)
                } else {
                    b.markdown.trim().to_string()
                };
                out.push_str(&format!(
                    "> **[사진 {n}]** {caption}\n\n![사진 {n}]({image_prefix}{shot}.png)\n\n"
                ));
            }
            BlockKind::Todo => {
                out.push_str(&format!(
                    "> ⚠️ {}\n\n",
                    b.markdown.trim().replace('\n', "\n> ")
                ));
            }
            _ => {
                out.push_str(b.markdown.trim());
                out.push_str("\n\n");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_images_and_marks_todos() {
        let blocks = vec![
            Block {
                id: "intro".into(),
                kind: BlockKind::Paragraph,
                markdown: "본문".into(),
                facts: vec!["F001".into()],
                shot: None,
            },
            Block {
                id: "shot".into(),
                kind: BlockKind::Image,
                markdown: "".into(),
                facts: vec![],
                shot: Some("menus/a".into()),
            },
            Block {
                id: "todo".into(),
                kind: BlockKind::Todo,
                markdown: "확인 필요: 비밀번호 규칙".into(),
                facts: vec![],
                shot: None,
            },
        ];
        let md = markdown("사용자 관리", &blocks, "../shots/", &|_| {
            "목록 화면".into()
        });
        assert!(md.starts_with("# 사용자 관리"));
        assert!(md.contains("<!-- blk:intro kind=paragraph facts=F001 -->"));
        assert!(
            md.contains("> **[사진 1]** 목록 화면")
                && md.contains("![사진 1](../shots/menus/a.png)")
        );
        assert!(md.contains("> ⚠️ 확인 필요: 비밀번호 규칙"));
    }
}
