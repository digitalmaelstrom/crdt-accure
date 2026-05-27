use serde::{Deserialize, Serialize};
use std::fmt;

pub type SiteId = String;

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Dot {
    pub site: SiteId,
    pub n: u64,
}

impl Dot {
    pub fn new(site: impl Into<SiteId>, n: u64) -> Self {
        Self { site: site.into(), n }
    }
}

impl fmt::Debug for Dot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.site, self.n)
    }
}

impl fmt::Display for Dot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.site, self.n)
    }
}
