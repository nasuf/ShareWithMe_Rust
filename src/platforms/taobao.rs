//! 淘宝 / 天猫 (Taobao / Tmall): product JSON-LD when the page is public. Most
//! product pages need a login or trip risk control, so the share-sheet title
//! and specification text is the reliable source.

use serde_json::Value;
use url::Url;

use crate::{extraction::structured_data, models::LinkMetadata};

use super::identity::query_value;

pub(super) const TAOBAO_ITEM_PREFIX: &str = "https://item.taobao.com/item.htm?id=";
pub(super) const TMALL_ITEM_PREFIX: &str = "https://detail.tmall.com/item.htm?id=";

pub(super) const MISSING_CONTENT_HINT: &str =
    "请保留 App 分享面板中的商品标题/规格文字；仅凭网页 URL 常会触发淘宝登录或风控。";

pub(super) fn identity(url: &Url, item_prefix: &str) -> Option<(String, String)> {
    let id = query_value(url, &["id", "itemId"])?;
    Some((id.clone(), format!("{item_prefix}{id}")))
}

pub(super) fn enrich_from_html(metadata: &mut LinkMetadata, json_ld: &[Value]) {
    structured_data::enrich_product(metadata, json_ld);
    structured_data::prefer_description_as_original(metadata);
}

#[cfg(test)]
mod tests {
    use url::Url;

    use super::{TMALL_ITEM_PREFIX, enrich_from_html, identity};
    use crate::{extraction::json_ld_values, models::LinkMetadata};

    #[test]
    fn product_description_becomes_the_body() {
        let html = r#"<script type="application/ld+json">{"@type":"Product","name":"保温杯","description":"316 不锈钢，500ml","offers":{"price":"89"}}</script>"#;
        let mut metadata = LinkMetadata::default();
        enrich_from_html(&mut metadata, &json_ld_values(html));
        assert_eq!(metadata.title, "保温杯");
        assert_eq!(metadata.original_text, "316 不锈钢，500ml");
    }

    #[test]
    fn reads_item_id_from_either_query_key() {
        let url = Url::parse("https://detail.m.tmall.com/item.htm?itemId=42&spm=x").unwrap();
        assert_eq!(
            identity(&url, TMALL_ITEM_PREFIX),
            Some((
                "42".to_string(),
                "https://detail.tmall.com/item.htm?id=42".to_string()
            ))
        );
    }
}
