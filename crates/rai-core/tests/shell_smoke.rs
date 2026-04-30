//! 実機 smoke test: `$SHELL` 経由で外部コマンドを起動できることを確認する。
//!
//! 検証対象は「`shell::user_shell_argv` が組み立てる `Command` が、起動先シェルに
//! ユーザーが期待する解決ロジックを使わせるか」。`type` はだいたいどのシェルにも
//! builtin として入っているので、それを呼んで成功すれば最低限の動作が取れている。

use rai_core::shell;

#[test]
fn user_shell_can_run_builtin_type() {
    let out = shell::user_shell_argv(&["type", "type"])
        .output()
        .expect("user shell should spawn");
    assert!(
        out.status.success(),
        "`type type` failed: exit={:?} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn detect_user_shell_returns_nonempty_path() {
    let (path, _) = shell::detect_user_shell();
    assert!(!path.is_empty(), "detect_user_shell returned empty path");
}

/// `$SHELL` を明示的に fish に向けたとき、fish 側の autoload function が
/// `shell::user_shell_argv` 経由で解決されることを確認する。fish が無い環境では skip。
///
/// `RAI_SMOKE_FISH_FUNC=<name>` を渡すと、その autoload function 名で確認する。
/// 未指定なら本テストは skip。
#[test]
fn user_shell_argv_resolves_fish_autoload_function() {
    let Ok(func) = std::env::var("RAI_SMOKE_FISH_FUNC") else {
        eprintln!("skip: RAI_SMOKE_FISH_FUNC not set");
        return;
    };
    let Ok(fish_path) = std::env::var("RAI_SMOKE_FISH_PATH") else {
        eprintln!("skip: RAI_SMOKE_FISH_PATH not set");
        return;
    };
    // SHELL を fish に強制してから shell::user_shell_argv を呼ぶ。
    std::env::set_var("SHELL", &fish_path);
    let out = shell::user_shell_argv(&["type", &func])
        .output()
        .expect("user shell should spawn");
    assert!(
        out.status.success(),
        "`type {func}` under SHELL={fish_path} failed: exit={:?} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}
