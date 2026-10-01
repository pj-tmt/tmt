//! Bounded new-executable plan and result contract for the native updater.
use serde_json::{Value, json};
use tmt_adapters::native_install::Product;

pub(crate) const LIMIT: usize = 64 * 1024;

pub(crate) struct Plan {
    pub products: Vec<Value>,
    pub pending: Vec<(Product, String, String)>,
}

impl Plan {
    pub fn document(&self) -> Value {
        let pending: Vec<_> = self
            .pending
            .iter()
            .map(|(product, version, selected)| {
                json!({
                    "product": product.as_str(),
                    "version": version,
                    "selected": selected,
                })
            })
            .collect();
        json!({ "products": self.products, "pending": pending })
    }

    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let value = decode(bytes)?;
        if value.as_object()?.len() != 2 {
            return None;
        }
        let products = rows(&value["products"])?;
        if products
            .iter()
            .any(|product| product["status"] == "changed")
        {
            return None;
        }
        let mut seen: Vec<_> = products
            .iter()
            .map(|row| row["product"].as_str().unwrap())
            .collect();
        let mut pending = Vec::new();
        for item in value["pending"].as_array()? {
            if item.as_object()?.len() != 3 {
                return None;
            }
            let name = item["product"].as_str()?;
            let product = product(name)?;
            if seen.contains(&name) {
                return None;
            }
            seen.push(name);
            pending.push((
                product,
                version(&item["version"])?.into(),
                version(&item["selected"])?.into(),
            ));
        }
        Some(Self { products, pending })
    }
}

fn decode(bytes: &[u8]) -> Option<Value> {
    (bytes.len() <= LIMIT)
        .then(|| serde_json::from_slice(bytes).ok())
        .flatten()
}

fn product(name: &str) -> Option<Product> {
    Product::ALL
        .into_iter()
        .find(|product| *product != Product::Cli && product.as_str() == name)
}

fn version(value: &Value) -> Option<&str> {
    let text = value.as_str()?;
    (!text.is_empty()
        && text.len() <= 128
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-+".contains(&byte)))
    .then_some(text)
}

fn rows(value: &Value) -> Option<Vec<Value>> {
    let rows = value.as_array()?;
    let mut seen = Vec::new();
    for row in rows {
        let object = row.as_object()?;
        let name = row["product"].as_str()?;
        product(name)?;
        if seen.contains(&name)
            || object.keys().any(|key| {
                ![
                    "product", "status", "version", "hint", "message", "error", "details",
                ]
                .contains(&key.as_str())
            })
        {
            return None;
        }
        seen.push(name);
        for key in ["hint", "message"] {
            if object.contains_key(key) && !row[key].is_string() {
                return None;
            }
        }
        match row["status"].as_str()? {
            "failed" => {
                if !row["error"]["code"].is_string() || !row["error"]["message"].is_string() {
                    return None;
                }
            }
            "changed" | "unchanged" | "skippedPinned" => {
                version(&row["version"])?;
            }
            _ => return None,
        }
    }
    Some(rows.clone())
}

pub(crate) fn parse_results(bytes: &[u8]) -> Option<Vec<Value>> {
    let value = decode(bytes)?;
    if value.as_object()?.len() != 1 {
        return None;
    }
    rows(&value["products"])
}
