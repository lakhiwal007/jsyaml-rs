//! The built-in tags and schemas of `src/schema.ts` and `src/tag/**`. Custom tags defined
//! in JS can't run here; the JS bindings send schemas containing them down the
//! events path instead (see `crates/jsyaml-node/js/index.cjs`).

/// What kind of node a tag constructs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagKind {
    /// Constructed from a scalar.
    Scalar,
    /// Constructed from a sequence.
    Sequence,
    /// Constructed from a mapping.
    Mapping,
}

/// One built-in tag definition; each variant names its upstream file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(missing_docs)] // the upstream file names are the docs
pub enum Tag {
    Str,         // tag/scalar/str.ts
    NullCore,    // tag/scalar/null_core.ts
    NullJson,    // tag/scalar/null_json.ts
    NullYaml11,  // tag/scalar/null_yaml11.ts
    BoolCore,    // tag/scalar/bool_core.ts
    BoolJson,    // tag/scalar/bool_json.ts
    BoolYaml11,  // tag/scalar/bool_yaml11.ts
    IntCore,     // tag/scalar/int_core.ts
    IntJson,     // tag/scalar/int_json.ts
    IntYaml11,   // tag/scalar/int_yaml11.ts
    FloatCore,   // tag/scalar/float_core.ts
    FloatJson,   // tag/scalar/float_json.ts
    FloatYaml11, // tag/scalar/float_yaml11.ts
    Merge,       // tag/scalar/merge.ts
    Binary,      // tag/scalar/binary.ts
    Timestamp,   // tag/scalar/timestamp.ts
    Seq,         // tag/sequence/seq.ts
    Omap,        // tag/sequence/omap.ts
    Pairs,       // tag/sequence/pairs.ts
    Map,         // tag/mapping/map.ts
    RealMap,     // tag/mapping/real_map.ts
    LegacyMap,   // tag/mapping/legacy_map.ts
    Set,         // tag/mapping/set.ts
}

impl Tag {
    /// Every built-in tag, in the order of upstream's `index.ts` exports.
    pub const ALL: [Tag; 23] = [
        Tag::Str,
        Tag::NullCore,
        Tag::NullJson,
        Tag::NullYaml11,
        Tag::BoolCore,
        Tag::BoolJson,
        Tag::BoolYaml11,
        Tag::IntCore,
        Tag::IntJson,
        Tag::IntYaml11,
        Tag::FloatCore,
        Tag::FloatJson,
        Tag::FloatYaml11,
        Tag::Merge,
        Tag::Binary,
        Tag::Timestamp,
        Tag::Seq,
        Tag::Omap,
        Tag::Pairs,
        Tag::Map,
        Tag::RealMap,
        Tag::LegacyMap,
        Tag::Set,
    ];

    /// The full tag name, e.g. `tag:yaml.org,2002:str`.
    pub fn name(self) -> &'static str {
        match self {
            Tag::Str => "tag:yaml.org,2002:str",
            Tag::NullCore | Tag::NullJson | Tag::NullYaml11 => "tag:yaml.org,2002:null",
            Tag::BoolCore | Tag::BoolJson | Tag::BoolYaml11 => "tag:yaml.org,2002:bool",
            Tag::IntCore | Tag::IntJson | Tag::IntYaml11 => "tag:yaml.org,2002:int",
            Tag::FloatCore | Tag::FloatJson | Tag::FloatYaml11 => "tag:yaml.org,2002:float",
            Tag::Merge => "tag:yaml.org,2002:merge",
            Tag::Binary => "tag:yaml.org,2002:binary",
            Tag::Timestamp => "tag:yaml.org,2002:timestamp",
            Tag::Seq => "tag:yaml.org,2002:seq",
            Tag::Omap => "tag:yaml.org,2002:omap",
            Tag::Pairs => "tag:yaml.org,2002:pairs",
            Tag::Map | Tag::RealMap | Tag::LegacyMap => "tag:yaml.org,2002:map",
            Tag::Set => "tag:yaml.org,2002:set",
        }
    }

    /// The upstream export name (`strTag`, `intCoreTag`, ...); the JS bindings use it to
    /// recognise built-in tag objects inside a user's schema.
    pub fn export_name(self) -> &'static str {
        match self {
            Tag::Str => "strTag",
            Tag::NullCore => "nullCoreTag",
            Tag::NullJson => "nullJsonTag",
            Tag::NullYaml11 => "nullYaml11Tag",
            Tag::BoolCore => "boolCoreTag",
            Tag::BoolJson => "boolJsonTag",
            Tag::BoolYaml11 => "boolYaml11Tag",
            Tag::IntCore => "intCoreTag",
            Tag::IntJson => "intJsonTag",
            Tag::IntYaml11 => "intYaml11Tag",
            Tag::FloatCore => "floatCoreTag",
            Tag::FloatJson => "floatJsonTag",
            Tag::FloatYaml11 => "floatYaml11Tag",
            Tag::Merge => "mergeTag",
            Tag::Binary => "binaryTag",
            Tag::Timestamp => "timestampTag",
            Tag::Seq => "seqTag",
            Tag::Omap => "omapTag",
            Tag::Pairs => "pairsTag",
            Tag::Map => "mapTag",
            Tag::RealMap => "realMapTag",
            Tag::LegacyMap => "legacyMapTag",
            Tag::Set => "setTag",
        }
    }

    /// Looks a tag up by [`Tag::export_name`].
    pub fn from_export_name(name: &str) -> Option<Tag> {
        Tag::ALL.into_iter().find(|t| t.export_name() == name)
    }

    /// The node kind this tag constructs.
    pub fn kind(self) -> TagKind {
        match self {
            Tag::Seq | Tag::Omap | Tag::Pairs => TagKind::Sequence,
            Tag::Map | Tag::RealMap | Tag::LegacyMap | Tag::Set => TagKind::Mapping,
            _ => TagKind::Scalar,
        }
    }
}

/// An ordered list of tags, like upstream `Schema`. Later tags with the same name and
/// kind replace earlier ones (`withTags` semantics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    tags: Vec<Tag>,
}

impl Schema {
    /// A schema from `tags`, applying replacement by name and kind.
    pub fn new(tags: impl IntoIterator<Item = Tag>) -> Self {
        Schema { tags: Vec::new() }.with_tags(tags)
    }

    /// `FAILSAFE_SCHEMA`
    pub fn failsafe() -> Self {
        Schema::new([Tag::Str, Tag::Seq, Tag::Map])
    }

    /// `JSON_SCHEMA`
    pub fn json() -> Self {
        Schema::failsafe().with_tags([Tag::NullJson, Tag::BoolJson, Tag::IntJson, Tag::FloatJson])
    }

    /// `CORE_SCHEMA`, the default.
    pub fn core() -> Self {
        Schema::failsafe().with_tags([Tag::NullCore, Tag::BoolCore, Tag::IntCore, Tag::FloatCore])
    }

    /// `YAML11_SCHEMA`
    pub fn yaml11() -> Self {
        // Keep in sync with YAML11_SCHEMA in upstream src/schema.ts.
        Schema::failsafe().with_tags([
            Tag::NullYaml11,
            Tag::BoolYaml11,
            Tag::IntYaml11,
            Tag::FloatYaml11,
            Tag::Timestamp,
            Tag::Merge,
            Tag::Binary,
            Tag::Omap,
            Tag::Pairs,
            Tag::Set,
        ])
    }

    /// `schema.withTags(...)`
    #[must_use]
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = Tag>) -> Self {
        for tag in tags {
            match self
                .tags
                .iter_mut()
                .find(|t| t.name() == tag.name() && t.kind() == tag.kind())
            {
                Some(slot) => *slot = tag,
                None => self.tags.push(tag),
            }
        }
        self
    }

    /// The tags, in resolution order.
    pub fn tags(&self) -> &[Tag] {
        &self.tags
    }

    /// The tag of `kind` named `name` (`lookupScalarTag` and friends; built-in tags never
    /// match by prefix).
    pub fn lookup(&self, kind: TagKind, name: &str) -> Option<Tag> {
        self.tags
            .iter()
            .copied()
            .find(|t| t.kind() == kind && t.name() == name)
    }
}

impl Default for Schema {
    fn default() -> Self {
        Schema::core()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_tags_replaces_same_name_and_kind() {
        let schema = Schema::core().with_tags([Tag::RealMap, Tag::Merge]);
        assert!(schema.tags().contains(&Tag::RealMap));
        assert!(!schema.tags().contains(&Tag::Map));
        assert_eq!(schema.tags().last(), Some(&Tag::Merge));
    }

    #[test]
    fn export_names_round_trip() {
        for tag in Tag::ALL {
            assert_eq!(Tag::from_export_name(tag.export_name()), Some(tag));
        }
    }
}
