use jsyaml::{ConstructorOptions, LoadOptions, ParserOptions, Schema, Tag};

/// The JS `LoadOptions` the native path supports. The schema arrives as the export names
/// of its built-in tags (`["strTag", "seqTag", ...]`), as found by `js/decode.cjs`.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Export names of the schema's tags, in order; empty means `CORE_SCHEMA`.
    pub tags: Vec<String>,
    /// `json`
    pub json: Option<bool>,
    /// `maxTotalMergeKeys`; `-1` disables the limit.
    pub max_total_merge_keys: Option<f64>,
    /// `maxAliases`; `-1` disables the limit.
    pub max_aliases: Option<f64>,
    /// `maxDepth`
    pub max_depth: Option<f64>,
}

impl Options {
    pub(crate) fn to_load_options(&self) -> Result<LoadOptions, String> {
        let schema = if self.tags.is_empty() {
            Schema::core()
        } else {
            let tags = self
                .tags
                .iter()
                .map(|name| {
                    Tag::from_export_name(name)
                        .ok_or_else(|| format!("unknown built-in tag {name}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Schema::new(tags)
        };
        let defaults = ConstructorOptions::default();
        Ok(LoadOptions {
            parser: ParserOptions {
                max_depth: self
                    .max_depth
                    .map_or(ParserOptions::default().max_depth, limit),
            },
            constructor: ConstructorOptions {
                schema,
                json: self.json.unwrap_or(defaults.json),
                max_total_merge_keys: self
                    .max_total_merge_keys
                    .map_or(defaults.max_total_merge_keys, unlimited),
                max_aliases: self.max_aliases.map_or(defaults.max_aliases, unlimited),
            },
        })
    }
}

#[allow(clippy::cast_sign_loss)] // clamped to >= 0 first
fn limit(n: f64) -> usize {
    if n.is_nan() {
        0
    } else {
        n.max(0.0).min(usize::MAX as f64) as usize
    }
}

fn unlimited(n: f64) -> Option<usize> {
    (n >= 0.0).then(|| limit(n))
}
