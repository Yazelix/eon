//! Namespace arbitration; lifecycle ownership remains with the selected lifetime.
use super::{
    Identity, MAX_GENERATIONS, cleanup_metadata, generation_candidates, generation_directory,
    generation_inventory, preferred_generation, valid_generation,
};
use crate::{
    cli::Inputs,
    control::{EndpointFailureKind, probe_generation_runtime},
    supervisor::{
        LifecycleLock, lock_supervisor_lifecycle, prepare_runtime, request_id,
        supervisor_lock_path, try_lock_supervisor_lifecycle, validate_private_directory,
    },
};
use std::{
    collections::hash_map::RandomState,
    fs::DirBuilder,
    hash::BuildHasher,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

pub(crate) struct NamespaceLock {
    root: PathBuf,
    _file: LifecycleLock,
}

impl NamespaceLock {
    pub(crate) fn acquire(root: &Path) -> Result<Self, String> {
        validate_private_directory(root)?;
        Ok(Self {
            root: root.into(),
            _file: lock_supervisor_lifecycle(&root.join("launch.lock"))?,
        })
    }

    pub(crate) fn inventory(&self) -> Result<Vec<(std::ffi::OsString, PathBuf)>, String> {
        let inventory = generation_inventory(&self.root)?;
        if inventory.count <= MAX_GENERATIONS {
            return inventory.bounded(&self.root);
        }
        for (id, runtime) in inventory.entries {
            let Some(id) = id.to_str().filter(|id| valid_generation(id)) else {
                continue;
            };
            if validate_private_directory(&runtime).is_err() {
                continue;
            }
            if let Ok(Some(_lifecycle)) =
                try_lock_supervisor_lifecycle(&supervisor_lock_path(&self.root, id))
            {
                // Existing cleanup retains every host, unknown object and unsafe record.
                let _ = cleanup_metadata(&runtime);
            }
        }
        generation_candidates(&self.root)
    }

    /// Metadata matches, excluding an existing owner's own lifetime when checking publication.
    pub(crate) fn matching(
        &self,
        build: &str,
        exclude: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let mut matches = Vec::new();
        for (id, runtime) in self.inventory()? {
            let id = id
                .to_str()
                .filter(|id| valid_generation(id))
                .ok_or_else(|| format!("invalid lifetime directory {}", runtime.display()))?;
            validate_private_directory(&runtime)?;
            let identity = Identity::read(&runtime)?;
            let active = identity
                .as_ref()
                .map_or(id, |record| record.active_build.as_str());
            if Some(id) == exclude || active != build {
                continue;
            }
            matches.push(id.into());
        }
        Ok(matches)
    }

    pub(crate) fn require_available(&self, build: &str, generation: &str) -> Result<(), String> {
        if self.matching(build, Some(generation))?.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "active build {build} already occupies another lifetime in this namespace"
            ))
        }
    }

    /// S2b's preparatory owner; public Update will consume it after adoption is implemented.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn publish_active(
        &self,
        expected: &Identity,
        next: &Identity,
        _lifecycle: &LifecycleLock,
    ) -> Result<(), String> {
        if next.generation != expected.generation || next.created_build != expected.created_build {
            return Err("active publication cannot change lifetime or creation build".into());
        }
        let runtime = generation_directory(&self.root, &expected.generation);
        if Identity::read(&runtime)?.as_ref() != Some(expected) {
            return Err("active publication does not match the committed identity".into());
        }
        self.require_available(&next.active_build, &expected.generation)?;
        next.publish(&runtime)
    }

    pub(crate) fn select(&self, build: &str) -> Result<Option<String>, String> {
        let matches = self.matching(build, None)?;
        if matches.len() <= 1 {
            // The normal launch probe validates the sole owner, including teardown races.
            return Ok(matches.into_iter().next());
        }
        let mut live = Vec::new();
        for id in &matches {
            let runtime = generation_directory(&self.root, id);
            let identity = Identity::read(&runtime)?;
            match probe_generation_runtime(&runtime.join("eon.sock")) {
                Ok(info) => {
                    if info.generation != *id
                        || info.active_build != build
                        || identity.as_ref().is_some_and(|record| {
                            !record.matches(
                                &info.generation,
                                &info.created_build,
                                &info.active_build,
                            )
                        })
                    {
                        return Err(format!(
                            "supervisor and identity disagree for lifetime {id}"
                        ));
                    }
                    live.push(id.clone());
                }
                Err(error) if error.kind == EndpointFailureKind::Dead => {}
                Err(error)
                    if matches!(
                        error.kind,
                        EndpointFailureKind::Unreachable | EndpointFailureKind::Starting
                    ) =>
                {
                    live.push(id.clone())
                }
                Err(error) => return Err(format!("cannot select lifetime {id}: {}", error.detail)),
            }
        }
        let matches = if live.is_empty() { matches } else { live };
        let candidates = matches
            .iter()
            .map(|id| (id.as_str(), build))
            .collect::<Vec<_>>();
        preferred_generation(build, &candidates)
            .map(|id| Some(id.into()))
            .ok_or_else(|| {
                format!(
                    "ambiguous lifetimes for active build {build}: {}",
                    matches.join(", ")
                )
            })
    }

    pub(crate) fn allocate(&self, inputs: &Inputs) -> Result<String, String> {
        let build = super::current_generation(inputs)?;
        prepare_runtime(&self.root.join("generations"))?;
        loop {
            let random = RandomState::new();
            let seed = request_id();
            let id = format!(
                "g1-{:016x}{:016x}",
                random.hash_one((&seed, 0)),
                random.hash_one((&seed, 1))
            );
            if id == build {
                continue;
            }
            match DirBuilder::new()
                .mode(0o700)
                .create(generation_directory(&self.root, &id))
            {
                Ok(()) => return Ok(id),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(format!("cannot allocate lifetime {id}: {error}")),
            }
        }
    }
}
