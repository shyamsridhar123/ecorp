use std::path::{Component, Path, PathBuf};

pub(super) fn path_is_inside(base: &Path, candidate: &str) -> bool {
    if contains_parent_path_segment(candidate) {
        return false;
    }
    path_is_inside_path(base, Path::new(candidate))
}

pub(super) fn path_is_inside_workspace(workspace: &Path, candidate: &str) -> bool {
    if contains_parent_path_segment(candidate) {
        return false;
    }
    let candidate =
        map_workspace_path(workspace, candidate).unwrap_or_else(|| PathBuf::from(candidate));
    path_is_inside_path(workspace, &candidate)
}

fn contains_parent_path_segment(candidate: &str) -> bool {
    candidate
        .split(['\\', '/'])
        .any(|component| component == "..")
}

fn path_is_inside_path(base: &Path, candidate: &Path) -> bool {
    let candidate = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        base.join(candidate)
    };
    let base = lexical_normalize(base);
    let candidate = lexical_normalize(&candidate);
    if !path_starts_with(&candidate, &base) {
        return false;
    }

    let Ok(canonical_base) = std::fs::canonicalize(&base) else {
        return false;
    };
    let Some(existing_ancestor) = nearest_existing_ancestor(&candidate) else {
        return false;
    };
    std::fs::canonicalize(existing_ancestor)
        .is_ok_and(|ancestor| path_starts_with(&ancestor, &canonical_base))
}

fn map_workspace_path(workspace: &Path, candidate: &str) -> Option<PathBuf> {
    let windows = candidate.replace('/', "\\");
    if windows.eq_ignore_ascii_case(r"C:\workspace") {
        return Some(workspace.to_path_buf());
    }
    const WINDOWS_PREFIX: &str = "C:\\workspace\\";
    if windows
        .get(..WINDOWS_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(WINDOWS_PREFIX))
    {
        return Some(join_workspace_alias(
            workspace,
            &windows[WINDOWS_PREFIX.len()..],
        ));
    }

    let unix = candidate.replace('\\', "/");
    if unix == "/workspace" {
        return Some(workspace.to_path_buf());
    }
    unix.strip_prefix("/workspace/")
        .map(|relative| join_workspace_alias(workspace, relative))
}

fn join_workspace_alias(workspace: &Path, relative: &str) -> PathBuf {
    relative
        .split(['\\', '/'])
        .filter(|component| !component.is_empty())
        .fold(workspace.to_path_buf(), |path, component| {
            path.join(component)
        })
}

fn nearest_existing_ancestor(path: &Path) -> Option<&Path> {
    let mut candidate = Some(path);
    while let Some(current) = candidate {
        if current.exists() {
            return Some(current);
        }
        candidate = current.parent();
    }
    None
}

#[cfg(windows)]
fn path_starts_with(path: &Path, base: &Path) -> bool {
    let path = path.components().collect::<Vec<_>>();
    let base = base.components().collect::<Vec<_>>();
    path.len() >= base.len()
        && path.iter().zip(base.iter()).all(|(candidate, expected)| {
            candidate
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&expected.as_os_str().to_string_lossy())
        })
}

#[cfg(not(windows))]
fn path_starts_with(path: &Path, base: &Path) -> bool {
    path.starts_with(base)
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

pub(super) fn bounded_text(value: &str, limit: usize) -> String {
    let mut bounded = value.chars().take(limit).collect::<String>();
    if value.chars().count() > limit {
        bounded.push('…');
    }
    bounded
}
