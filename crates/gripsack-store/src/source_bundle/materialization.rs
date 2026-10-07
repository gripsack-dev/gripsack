//! Native overlays consume resolved captured objects, not aliases whose targets
//! might be absent from a selected overlay. Evaluation retains alias identity.
use super::{
    CaptureBudget, SourceBundle, SourceObject,
    inventory::{MAX_DEPTH, invalid},
};
use crate::hash::{PayloadHash, canonical_file_hash};
use std::{
    collections::BTreeMap,
    io,
    path::{Component, Path, PathBuf},
};

impl SourceBundle {
    /// Resolve through the retained root capability, including admitted SDK
    /// roots. The returned path never names a live original source.
    pub fn materialization_path(&self, relative: &Path) -> io::Result<PathBuf> {
        if relative.components().take(MAX_DEPTH + 1).count() > MAX_DEPTH {
            return Err(invalid("materialized source exceeds its path-depth budget"));
        }
        if relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
        {
            return Err(invalid("materialized source must be repository-relative"));
        }
        if self.excluded_selection(relative) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!(
                "repository source {} is excluded by capture policy", relative.display()
            )));
        }
        let resolved = self.root.canonicalize(Path::new("repo").join(relative))?;
        Ok(self
            .repository
            .parent()
            .expect("captured root exists")
            .join(resolved))
    }

    /// An alias to an included parent must not turn a deliberately unavailable
    /// child into an ordinary missing selector eligible for artifact fallback.
    fn excluded_selection(&self, relative: &Path) -> bool {
        if self.capture_policy.exclusions().is_empty() {
            return false;
        }
        let mut logical = PathBuf::from("repo");
        for component in relative.components().filter(|part| *part != Component::CurDir) {
            logical.push(component.as_os_str());
            if let Ok(relative) = logical.strip_prefix("repo")
                && self.capture_policy.excludes(relative)
            {
                return true;
            }
            if let Some(SourceObject::Alias { target }) = logical.to_str()
                .and_then(|path| self.inventory.entry(path))
                .map(|entry| &entry.object)
            {
                logical = PathBuf::from(target);
            }
        }
        false
    }

    /// Visit a bounded resolved selection. Aliased directories may expand, so
    /// count/byte/depth budgets apply again to the materialized logical paths.
    /// Missing selections remain artifact-supplied inputs, as for direct IR.
    pub fn visit_materialized<'a>(
        &self,
        froms: impl IntoIterator<Item = &'a str>,
        mut visit: impl FnMut(&Path, &Path, &SourceObject) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut budget = CaptureBudget::default();
        for from in froms {
            let relative: PathBuf = Path::new(from)
                .components()
                .filter(|part| *part != Component::CurDir)
                .collect();
            match self.materialization_path(&relative) {
                Ok(path) => {
                    self.visit_materialized_one(&relative, &path, &mut budget, &mut visit)?
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn visit_materialized_one(
        &self,
        relative: &Path,
        physical: &Path,
        budget: &mut CaptureBudget,
        visit: &mut impl FnMut(&Path, &Path, &SourceObject) -> io::Result<()>,
    ) -> io::Result<()> {
        if relative.components().count() > MAX_DEPTH {
            return Err(invalid("materialized source exceeds its path-depth budget"));
        }
        let logical = relative
            .to_str()
            .ok_or_else(|| invalid("materialized source is not UTF-8"))?;
        budget.entry(logical)?;
        let captured = physical
            .strip_prefix(self.repository.parent().expect("captured root exists"))
            .map_err(|_| invalid("materialized source escaped captured roots"))?;
        let captured = captured
            .to_str()
            .ok_or_else(|| invalid("captured source is not UTF-8"))?;
        let object = &self
            .inventory
            .entry(captured)
            .ok_or_else(|| invalid("materialized source is not inventoried"))?
            .object;
        match object {
            SourceObject::File { bytes, .. } => {
                budget.content(
                    usize::try_from(bytes.bytes())
                        .map_err(|_| invalid("source length exceeds native domain"))?,
                )?;
                visit(relative, physical, object)
            }
            SourceObject::Directory => {
                visit(relative, physical, object)?;
                for child in std::fs::read_dir(physical)? {
                    let child = child?;
                    let relative = relative.join(child.file_name());
                    let path = self.materialization_path(&relative)?;
                    self.visit_materialized_one(&relative, &path, budget, visit)?;
                }
                Ok(())
            }
            SourceObject::Alias { .. } => {
                Err(invalid("materialization did not resolve a captured alias"))
            }
        }
    }

    /// Same canonical tree identity as the resolved overlay later staged by the
    /// executor. Parent directories are synthesized, without materializing data.
    pub fn overlay_hash(&self, froms: &[String]) -> io::Result<PayloadHash> {
        let mut entries = BTreeMap::new();
        let mut budget = CaptureBudget::default();
        let directory = crate::hash::directory_entry_hash();
        self.visit_materialized(froms.iter().map(String::as_str), |relative, physical, _| {
            if !relative.as_os_str().is_empty() {
                let name = relative
                    .to_str()
                    .ok_or_else(|| invalid("source path is not UTF-8"))?;
                if !entries.contains_key(name) {
                    budget.entry(name)?;
                    entries.insert(
                        name.to_owned(),
                        String::from(canonical_file_hash(physical)?),
                    );
                }
            }
            for parent in relative
                .ancestors()
                .skip(1)
                .filter(|path| !path.as_os_str().is_empty())
            {
                let name = parent
                    .to_str()
                    .ok_or_else(|| invalid("source ancestor is not UTF-8"))?;
                if !entries.contains_key(name) {
                    budget.entry(name)?;
                    entries.insert(name.to_owned(), directory.clone());
                }
            }
            Ok(())
        })?;
        Ok(crate::hash::hash_sorted_entries(entries))
    }
}
