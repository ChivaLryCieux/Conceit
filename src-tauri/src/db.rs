//! SQLite 连接与 schema 迁移。
//!
//! 书库目录下除 `manga/`（图片）与 `thumbs/`（封面缩略图）外，新增
//! `library.db`：书架索引（条目、排序、页列表、尺寸、阅读进度）从这里读写，
//! 取代原先整体序列化的 `index.json`。`index.json` 只在首次启动时读取一次，
//! 搬进数据库后重命名为 `index.json.migrated` 留底（见
//! [`crate::library::migrate_json_if_present`]）。
//!
//! schema 按版本演进：`migrations/*.sql` 用 `include_str!` 编译期内嵌（安卓包体里
//! 没有散落的 .sql 文件，也不存在运行时路径解析），启动时对照
//! `PRAGMA user_version` 补齐未应用的迁移，每个迁移一个事务。

use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};

/// 书库数据库文件名（位于书库根目录，与原 index.json 同级）
pub const DB_FILE_NAME: &str = "library.db";

/// 按顺序排列的 schema 迁移：版本号 -> SQL。新迁移追加到末尾，版本号必须递增。
const MIGRATIONS: &[(i64, &str)] = &[(1, include_str!("../migrations/001_init.sql"))];

/// 打开（或创建）书库数据库，设置连接参数并应用缺失的 schema 迁移。
///
/// 库文件损坏（异常断电 / 存储故障的残留）时改名留底、以空库重启：
/// 图片文件不受影响，书库体检的「孤儿目录收编」能把漫画一部部捞回书架；
/// 更彻底的恢复是把 `index.json.migrated` 改回 `index.json` 再启动，
/// 迁移逻辑会把它重新搬进空库（README「书库目录结构」有说明）。
pub fn open(path: &Path) -> Result<Connection, String> {
    match open_and_migrate(path) {
        Ok(conn) => Ok(conn),
        Err(e) if is_corruption(&e) => {
            let backup = quarantine(path);
            eprintln!(
                "数据库损坏（已留底 {}，以空库启动）: {e}",
                backup.display()
            );
            open_and_migrate(path)
        }
        Err(e) => Err(e),
    }
}

fn open_and_migrate(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path)
        .map_err(|e| format!("打开数据库失败 {}: {e}", path.display()))?;
    apply_pragmas(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

/// 连接级参数（不落库，每次打开都要设）：
/// - `journal_mode=WAL`：读写不互斥、进程崩溃不坏库；
/// - `synchronous=NORMAL`：WAL 下进程崩溃仍安全，只有断电可能丢最后几笔事务，
///   对书架索引可接受（图片是普通文件，本来就不受事务保护）；
/// - `foreign_keys=ON`：SQLite 默认关闭外键，`pages.manga_id` 的级联删除
///   必须每个连接显式打开才生效。
fn apply_pragmas(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA foreign_keys=ON;
         PRAGMA synchronous=NORMAL;",
    )
    .map_err(|e| format!("设置数据库参数失败: {e}"))
}

/// 应用所有缺失的迁移。每个迁移在独立事务里执行，且把 `user_version` 写进
/// 同一事务：提交才算迁移完成，中途失败或被 kill 会整体回滚，不会留下
/// 「一半表建好」的中间态；重跑同一迁移也只会看到它已经完成。
fn migrate(conn: &Connection) -> Result<(), String> {
    let current = schema_version(conn)?;
    for (version, sql) in MIGRATIONS.iter().filter(|(v, _)| *v > current) {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("开始 schema 迁移事务失败: {e}"))?;
        tx.execute_batch(sql)
            .map_err(|e| format!("应用 schema 迁移 v{version} 失败: {e}"))?;
        tx.execute_batch(&format!("PRAGMA user_version = {version};"))
            .map_err(|e| format!("记录 schema 版本失败: {e}"))?;
        tx.commit()
            .map_err(|e| format!("提交 schema 迁移 v{version} 失败: {e}"))?;
    }
    Ok(())
}

/// 当前 schema 版本（`PRAGMA user_version`；新建库为 0）
pub fn schema_version(conn: &Connection) -> Result<i64, String> {
    conn.query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| format!("读取 schema 版本失败: {e}"))
}

/// 损坏的库文件改名留底，连 -wal / -shm 一起移走（留着旧的 WAL 会干扰新库）。
/// 返回留底路径；改名失败不影响调用方以空库重启。
fn quarantine(path: &Path) -> PathBuf {
    let stamp = crate::library::now_unix();
    let backup = path.with_extension(format!("db.corrupt.{stamp}"));
    for (from, to) in sidecar_paths(path, &backup) {
        if from.exists() {
            let _ = fs::rename(&from, &to);
        }
    }
    backup
}

/// 主库与 -wal / -shm 的（源, 目标）配对
fn sidecar_paths(path: &Path, backup: &Path) -> Vec<(PathBuf, PathBuf)> {
    let mut pairs = vec![(path.to_path_buf(), backup.to_path_buf())];
    for suffix in ["-wal", "-shm"] {
        pairs.push((sidecar(path, suffix), sidecar(backup, suffix)));
    }
    pairs
}

/// 在路径后拼后缀（library.db -> library.db-wal），保留所在目录
fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let name = path.file_name().unwrap_or_default();
    path.with_file_name(format!("{}{suffix}", name.to_string_lossy()))
}

/// SQLite 的「文件不是数据库 / 镜像损坏」类错误——这类值得弃库重启；
/// 权限不足、磁盘满等原因不在此列，原样上报让用户看到。
fn is_corruption(err: &str) -> bool {
    err.contains("not a database")
        || err.contains("database disk image is malformed")
        || err.contains("file is encrypted")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 测试用：内存库 + 完整迁移（schema 状态与 open() 一致）。
    /// journal_mode=WAL 对内存库是空操作（返回 memory），不影响其他参数。
    pub(crate) fn open_in_memory() -> Result<Connection, String> {
        let conn = Connection::open_in_memory()
            .map_err(|e| format!("打开内存数据库失败: {e}"))?;
        apply_pragmas(&conn)?;
        migrate(&conn)?;
        Ok(conn)
    }

    fn table_names(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        rows.flatten().collect()
    }

    #[test]
    fn migrate_creates_schema_and_sets_version() {
        let conn = open_in_memory().expect("迁移应成功");
        assert_eq!(schema_version(&conn).unwrap(), 1);
        assert_eq!(
            table_names(&conn),
            vec!["container_order", "items", "meta", "pages"]
        );
        // 新库的 meta 表是空的（next_seq 由首次 save 写入）
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM meta", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn migrate_is_idempotent() {
        let conn = open_in_memory().unwrap();
        // 再跑一遍同样的迁移：版本号已到位，应直接跳过而不重复建表
        migrate(&conn).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), 1);
        assert_eq!(table_names(&conn).len(), 4);
    }

    #[test]
    fn migration_rolls_back_on_failure() {
        // 模拟迁移中途失败：版本号与建表在同一个事务里，必须一起回滚。
        // 手工把版本推到 1 之前的状态行不通，这里直接验证事务语义：
        // 在一个会失败的事务里建表 + 设版本，回滚后两者都不该留下痕迹。
        let conn = open_in_memory().unwrap();
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute_batch("CREATE TABLE should_vanish (x);").unwrap();
        tx.execute_batch("PRAGMA user_version = 99;").unwrap();
        drop(tx); // 未提交即 drop = 回滚
        assert_eq!(schema_version(&conn).unwrap(), 1);
        assert!(!table_names(&conn).contains(&"should_vanish".to_string()));
    }

    #[test]
    fn foreign_keys_cascade_pages_on_item_delete() {
        let conn = open_in_memory().unwrap();
        conn.execute_batch(
            "INSERT INTO items (id, kind, title, show_cover, added_at, progress)
             VALUES ('m1', 'manga', '甲', 0, 0, 0.0);
             INSERT INTO pages (manga_id, path, pos) VALUES ('m1', '001.jpg', 0);",
        )
        .unwrap();
        conn.execute("DELETE FROM items WHERE id = 'm1'", []).unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM pages", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0, "条目删除后页记录应级联消失");
    }

    #[test]
    fn open_creates_file_db_and_quarantines_garbage() {
        let dir = std::env::temp_dir().join(format!("conceit-db-{}", crate::library::now_unix()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(DB_FILE_NAME);

        // 首次打开：建文件、跑迁移
        let conn = open(&path).unwrap();
        assert_eq!(schema_version(&conn).unwrap(), 1);
        drop(conn);

        // 塞入非数据库内容：再打开应改名留底并以空库重启，而不是报错
        fs::write(&path, b"this is not a sqlite database at all").unwrap();
        let conn = open(&path).expect("损坏的库应被隔离并以空库重启");
        assert_eq!(schema_version(&conn).unwrap(), 1);
        drop(conn);

        // 留底文件存在，且没有残留的 -wal / -shm 干扰新库
        let backups: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            backups.iter().any(|n| n.starts_with("library.db.corrupt.")),
            "应留下损坏备份，实际: {backups:?}"
        );
        assert!(!backups.iter().any(|n| n.ends_with("-wal") || n.ends_with("-shm")));

        fs::remove_dir_all(&dir).ok();
    }
}
