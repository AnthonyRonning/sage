//! Filesystem tools for Sage
//! 
//! Allows Sage to read, write, and manage files within its workspace.

use crate::ToolResult;
use std::path::Path;

/// Read the contents of a file
pub async fn read_file(path: &Path) -> ToolResult {
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => ToolResult::success(contents),
        Err(e) => ToolResult::error(format!("Failed to read file: {}", e)),
    }
}

/// Write contents to a file
pub async fn write_file(path: &Path, contents: &str) -> ToolResult {
    match tokio::fs::write(path, contents).await {
        Ok(()) => ToolResult::success(format!("Wrote {} bytes to {}", contents.len(), path.display())),
        Err(e) => ToolResult::error(format!("Failed to write file: {}", e)),
    }
}

/// List contents of a directory
pub async fn list_directory(path: &Path) -> ToolResult {
    match tokio::fs::read_dir(path).await {
        Ok(mut entries) => {
            let mut items = Vec::new();
            while let Ok(Some(entry)) = entries.next_entry().await {
                let name = entry.file_name().to_string_lossy().to_string();
                let file_type = entry.file_type().await.ok();
                let type_str = match file_type {
                    Some(ft) if ft.is_dir() => "dir",
                    Some(ft) if ft.is_file() => "file",
                    Some(ft) if ft.is_symlink() => "link",
                    _ => "unknown",
                };
                items.push(format!("{} ({})", name, type_str));
            }
            ToolResult::success(items.join("\n"))
        }
        Err(e) => ToolResult::error(format!("Failed to list directory: {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_list_directory() {
        let result = list_directory(&PathBuf::from(".")).await;
        assert!(result.success);
    }
}
