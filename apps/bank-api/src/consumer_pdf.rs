//! Small, self-contained printable statement. Only immutable native facts enter the document.
use serde_json::Value;
fn literal(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '?'
            }
        })
        .fold(String::new(), |mut out, c| {
            if matches!(c, '(' | ')' | '\\') {
                out.push('\\');
            }
            out.push(c);
            out
        })
}
pub fn statement(document: &Value) -> Vec<u8> {
    let field = |key: &str| document[key].as_str().unwrap_or("");
    let mut lines = vec![
        "PIGGYBANK - SYNTHETIC TST STATEMENT".to_owned(),
        "TST has no monetary value. Not a tax form.".into(),
        format!("Account: {}", field("accountId")),
        format!(
            "Period: {} inclusive to {} exclusive",
            field("from"),
            field("to")
        ),
        format!("Native statement: {}", field("id")),
        format!("Source SHA-256: {}", field("digest")),
        format!("Opening minor TST: {}", field("openingMinor")),
        format!("Closing minor TST: {}", field("closingMinor")),
        String::new(),
    ];
    if let Some(items) = document["items"].as_array() {
        for item in items {
            lines.push(format!(
                "{} | {} | minor TST {}",
                item["date"].as_str().unwrap_or(""),
                item["kind"].as_str().unwrap_or(""),
                item["deltaMinor"].as_str().unwrap_or("")
            ));
            lines.push(format!(
                "Native batch: {}",
                item["batchId"].as_str().unwrap_or("")
            ));
        }
    }
    render(&lines)
}
/// WP-25 period statement version: exact opening, signed movements, closing and linked corrections.
pub fn period_statement(
    document: &Value,
    content_digest: &str,
    superseded_by: Option<&str>,
) -> Vec<u8> {
    let text = |v: &Value| {
        v.as_str()
            .map(String::from)
            .unwrap_or_else(|| v.to_string())
    };
    let d = |key: &str| text(&document[key]);
    let t = |key: &str| text(&document["totals"][key]);
    let mut lines = vec![
        "PIGGYBANK - SYNTHETIC TST PERIOD STATEMENT".to_owned(),
        "TST has no monetary value. Not a tax form.".into(),
        format!("Account: {}", d("accountId")),
        format!(
            "Period: {} inclusive to {} exclusive (UTC)",
            d("periodStart"),
            d("periodEnd")
        ),
        format!("Version {} ({}) id {}", d("version"), d("reason"), d("id")),
        format!("Content SHA-256: {content_digest}"),
        format!("Snapshot SHA-256: {}", d("snapshotDigest")),
        superseded_by
            .map(|s| format!("SUPERSEDED by version id {s}"))
            .unwrap_or_else(|| "Current version".into()),
        String::new(),
        format!("Opening minor TST: {}", d("openingMinor")),
        format!("Credits minor TST: {}", t("creditsMinor")),
        format!("Debits minor TST: {}", t("debitsMinor")),
        format!(
            "Fees minor TST: {}  Interest minor TST: {}",
            t("feesMinor"),
            t("interestMinor")
        ),
        format!("Closing minor TST: {}", d("closingMinor")),
        String::new(),
    ];
    for item in document["items"].as_array().into_iter().flatten() {
        lines.push(format!(
            "{} | {} | minor TST {}",
            text(&item["businessDate"]),
            text(&item["kind"]),
            text(&item["deltaMinor"])
        ));
        lines.push(format!("Native batch: {}", text(&item["batchId"])));
    }
    let linked = document["linkedCorrections"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !linked.is_empty() {
        lines.push(String::new());
        lines.push("Linked corrections booked after this period (not in totals):".into());
        for item in linked {
            lines.push(format!(
                "{} | minor TST {} | corrects {}",
                text(&item["businessDate"]),
                text(&item["deltaMinor"]),
                text(&item["originalBatchId"])
            ));
        }
    }
    lines.push(String::new());
    lines.push(format!(
        "Retain until {} under fixture policy v{}",
        text(&document["retention"]["retainUntil"]),
        text(&document["retention"]["policyVersion"])
    ));
    render(&lines)
}
fn render(lines: &[String]) -> Vec<u8> {
    let pages = lines.chunks(44).collect::<Vec<_>>();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        String::new(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>".into(),
    ];
    let mut kids = Vec::new();
    for (n, page) in pages.iter().enumerate() {
        let id = objects.len() + 1;
        kids.push(format!("{id} 0 R"));
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",id+1));
        let mut content = format!(
            "BT /F1 9 Tf 36 756 Td 15 TL (Page {} of {}) Tj T* ",
            n + 1,
            pages.len()
        );
        for line in *page {
            content.push_str(&format!("({}) Tj T* ", literal(line)));
        }
        content.push_str("ET");
        objects.push(format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            content.len(),
            content
        ));
    }
    objects[1] = format!(
        "<< /Type /Pages /Count {} /Kids [{}] >>",
        kids.len(),
        kids.join(" ")
    );
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (n, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", n + 1, object).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.iter().skip(1) {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            offsets.len(),
            xref
        )
        .as_bytes(),
    );
    out
}
#[cfg(test)]
mod tests {
    #[test]
    fn pdf_retains_every_source_fact_and_escapes_literals() {
        let items=(0..100).map(|i|serde_json::json!({"batchId":format!("native-{i}"),"date":"2026-11-16","kind":"TEST(escaped)","deltaMinor":"-1"})).collect::<Vec<_>>();
        let pdf = super::statement(&serde_json::json!({"items":items}));
        let s = String::from_utf8(pdf).unwrap();
        assert!(s.contains("/Count 5"));
        assert!(s.contains("native-99"));
        assert!(s.contains("TEST\\(escaped\\)"));
        let xref: usize = s
            .split("startxref\n")
            .nth(1)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(s[xref..].starts_with("xref\n"));
    }
}
