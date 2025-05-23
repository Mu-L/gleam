#[cfg(test)]
mod tests;

use std::{collections::HashSet, time::SystemTime};

use camino::{Utf8Path, Utf8PathBuf};

use ecow::EcoString;
use serde::{Deserialize, Serialize};

use super::{
    Mode, Origin, SourceFingerprint, Target,
    package_compiler::{CacheMetadata, CachedModule, Input, UncompiledModule},
    package_loader::{CodegenRequired, GleamFile},
};
use crate::{
    Error, Result,
    error::{FileIoAction, FileKind},
    io::{CommandExecutor, FileSystemReader, FileSystemWriter},
    warning::{TypeWarningEmitter, WarningEmitter},
};

#[derive(Debug)]
pub(crate) struct ModuleLoader<'a, IO> {
    pub io: IO,
    pub warnings: &'a WarningEmitter,
    pub mode: Mode,
    pub target: Target,
    pub codegen: CodegenRequired,
    pub package_name: &'a EcoString,
    pub artefact_directory: &'a Utf8Path,
    pub origin: Origin,
    /// The set of modules that have had partial compilation done since the last
    /// successful compilation.
    pub incomplete_modules: &'a HashSet<EcoString>,
}

impl<'a, IO> ModuleLoader<'a, IO>
where
    IO: FileSystemReader + FileSystemWriter + CommandExecutor + Clone,
{
    /// Load a module from the given path.
    ///
    /// If the module has been compiled before and the source file has not been
    /// changed since then, load the precompiled data instead.
    ///
    /// Whether the module has changed or not is determined by comparing the
    /// modification time of the source file with the value recorded in the
    /// `.timestamp` file in the artefact directory.
    pub fn load(&self, file: GleamFile) -> Result<Input> {
        let name = file.module_name.clone();
        let source_mtime = self.io.modification_time(&file.path)?;

        let read_source = |name| self.read_source(file.path.clone(), name, source_mtime);

        let meta = match self.read_cache_metadata(&file)? {
            Some(meta) => meta,
            None => return read_source(name).map(Input::New),
        };

        // The cache currently does not contain enough data to perform codegen,
        // so if codegen is required in this compiler run then we must check
        // that codegen has already been performed before using a cache.
        if self.codegen.is_required() && !meta.codegen_performed {
            tracing::debug!(?name, "codegen_required_cache_insufficient");
            return read_source(name).map(Input::New);
        }

        // If the timestamp of the source is newer than the cache entry and
        // the hash of the source differs from the one in the cache entry,
        // then we need to recompile.
        if meta.mtime < source_mtime {
            let source_module = read_source(name.clone())?;
            if meta.fingerprint != SourceFingerprint::new(&source_module.code) {
                tracing::debug!(?name, "cache_stale");
                return Ok(Input::New(source_module));
            } else if self.mode == Mode::Lsp && self.incomplete_modules.contains(&name) {
                // Since the lsp can have valid but incorrect intermediate code states between
                // successful compilations, we need to invalidate the cache even if the fingerprint matches
                tracing::debug!(?name, "cache_stale for lsp");
                return Ok(Input::New(source_module));
            }
        }

        Ok(Input::Cached(self.cached(file, meta)))
    }

    /// Read the cache metadata file from the artefact directory for the given
    /// source file. If the file does not exist, return `None`.
    fn read_cache_metadata(&self, source_file: &GleamFile) -> Result<Option<CacheMetadata>> {
        let meta_path = source_file.cache_files(&self.artefact_directory).meta_path;

        if !self.io.is_file(&meta_path) {
            return Ok(None);
        }

        let binary = self.io.read_bytes(&meta_path)?;
        let cache_metadata = CacheMetadata::from_binary(&binary).map_err(|e| -> Error {
            Error::FileIo {
                action: FileIoAction::Parse,
                kind: FileKind::File,
                path: meta_path,
                err: Some(e),
            }
        })?;
        Ok(Some(cache_metadata))
    }

    fn read_source(
        &self,
        path: Utf8PathBuf,
        name: EcoString,
        mtime: SystemTime,
    ) -> Result<UncompiledModule, Error> {
        read_source(
            self.io.clone(),
            self.target,
            self.origin,
            path,
            name,
            self.package_name.clone(),
            mtime,
            self.warnings.clone(),
        )
    }

    fn cached(&self, file: GleamFile, meta: CacheMetadata) -> CachedModule {
        CachedModule {
            dependencies: meta.dependencies,
            source_path: file.path,
            origin: self.origin,
            name: file.module_name,
            line_numbers: meta.line_numbers,
        }
    }
}

pub(crate) fn read_source<IO>(
    io: IO,
    target: Target,
    origin: Origin,
    path: Utf8PathBuf,
    name: EcoString,
    package_name: EcoString,
    mtime: SystemTime,
    emitter: WarningEmitter,
) -> Result<UncompiledModule>
where
    IO: FileSystemReader + FileSystemWriter + CommandExecutor + Clone,
{
    let code: EcoString = io.read(&path)?.into();

    let parsed = crate::parse::parse_module(path.clone(), &code, &emitter).map_err(|error| {
        Error::Parse {
            path: path.clone(),
            src: code.clone(),
            error: Box::new(error),
        }
    })?;
    let mut ast = parsed.module;
    let extra = parsed.extra;
    let dependencies = ast.dependencies(target);

    ast.name = name.clone();
    let module = UncompiledModule {
        package: package_name,
        dependencies,
        origin,
        extra,
        mtime,
        path,
        name,
        code,
        ast,
    };
    Ok(module)
}
