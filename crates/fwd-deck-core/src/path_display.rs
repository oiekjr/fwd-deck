use std::{
    env,
    path::{MAIN_SEPARATOR, Path},
};

/// Path をユーザー表示用文字列へ変換する
pub fn format_path_for_display(path: &Path) -> String {
    let Some(home) = env::var_os("HOME") else {
        return path.display().to_string();
    };

    format_path_for_display_with_home(path, Path::new(&home))
}

/// 指定されたホームディレクトリを基準に Path をユーザー表示用文字列へ変換する
pub fn format_path_for_display_with_home(path: &Path, home: &Path) -> String {
    if home.as_os_str().is_empty() {
        return path.display().to_string();
    }

    let Ok(relative_path) = path.strip_prefix(home) else {
        return path.display().to_string();
    };

    if relative_path.as_os_str().is_empty() {
        return "~".to_owned();
    }

    format!("~{MAIN_SEPARATOR}{}", relative_path.display())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// ホームディレクトリ自身をチルダへ変換することを検証する
    #[test]
    fn format_path_for_display_with_home_shortens_home_itself() {
        let home = Path::new("/Users/example");

        let result = format_path_for_display_with_home(home, home);

        assert_eq!(result, "~");
    }

    /// ホームディレクトリ配下のパスをチルダ始まりへ変換することを検証する
    #[test]
    fn format_path_for_display_with_home_shortens_home_child() {
        let home = Path::new("/Users/example");
        let path = Path::new("/Users/example/projects/fwd-deck");

        let result = format_path_for_display_with_home(path, home);

        assert_eq!(result, "~/projects/fwd-deck");
    }

    /// ホームディレクトリと同じ接頭辞を持つ兄弟パスを短縮しないことを検証する
    #[test]
    fn format_path_for_display_with_home_keeps_sibling_path() {
        let home = Path::new("/Users/example");
        let path = Path::new("/Users/example-work/fwd-deck");

        let result = format_path_for_display_with_home(path, home);

        assert_eq!(result, "/Users/example-work/fwd-deck");
    }

    /// 相対パスを短縮しないことを検証する
    #[test]
    fn format_path_for_display_with_home_keeps_relative_path() {
        let home = Path::new("/Users/example");
        let path = Path::new("fwd-deck.toml");

        let result = format_path_for_display_with_home(path, home);

        assert_eq!(result, "fwd-deck.toml");
    }

    /// ホームディレクトリ未設定相当の場合に短縮しないことを検証する
    #[test]
    fn format_path_for_display_with_home_keeps_path_when_home_is_empty() {
        let path = Path::new("/Users/example/projects/fwd-deck");

        let result = format_path_for_display_with_home(path, Path::new(""));

        assert_eq!(result, "/Users/example/projects/fwd-deck");
    }
}
