-- 001_init.sql — Conceit 书库 schema 初始版本。
--
-- 这里只存元数据：图片文件仍在 manga/<id>/ 与 thumbs/ 下（见 README「书库目录结构」），
-- 数据库记录的是「书架上有哪些条目、每部漫画有哪些页、页的顺序与尺寸、阅读进度」，
-- 即原先 index.json 的全部内容。图片字节不进库，坏一个文件最多损失一部漫画，
-- 不会让整个书库不可读。
--
-- 表结构与原 LibraryIndex 的字段一一对应，首次启动时逐字段搬进来：
--   items            <- Item（id / kind / title / parent / show_cover / added_at）
--                       + MangaMeta 的 dir / cover / thumb / progress（folder 条目为 NULL）
--   pages            <- MangaMeta.pages（自然排序后的顺序）+ dims（宽高，可空）
--   container_order  <- LibraryIndex.order（容器 -> 有序子条目 id 列表）
--   meta             <- LibraryIndex.next_seq 等不便拆列的杂项
--
-- 由 src/db.rs 的迁移 runner 在事务里执行；PRAGMA user_version 记版本。

CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE items (
    id         TEXT PRIMARY KEY,
    kind       TEXT    NOT NULL CHECK (kind IN ('manga', 'folder')),
    title      TEXT    NOT NULL,
    parent     TEXT,                        -- NULL = 根目录
    show_cover INTEGER NOT NULL DEFAULT 0,  -- 0 / 1
    added_at   INTEGER NOT NULL,            -- Unix 秒
    -- 以下四列仅 kind = 'manga' 有值；folder 条目保持 NULL
    dir        TEXT,                        -- 漫画目录，如 manga/m1
    cover      TEXT,                        -- 封面页路径（漫画目录内的相对路径）
    thumb      TEXT,                        -- 封面缩略图路径（相对书库根目录）
    progress   REAL    NOT NULL DEFAULT 0   -- 阅读进度 0.0 - 1.0
);

CREATE TABLE pages (
    manga_id TEXT    NOT NULL,
    path     TEXT    NOT NULL,  -- 漫画目录内的完整相对路径（递归扫描可能含子目录，如 sub/003.jpg）
    pos      INTEGER NOT NULL,  -- 自然排序后的位置；读回时按它恢复 pages 的顺序
    width    INTEGER,           -- 像素宽；读不出尺寸的页为 NULL，前端回退估算宽高比
    height   INTEGER,
    PRIMARY KEY (manga_id, path),
    FOREIGN KEY (manga_id) REFERENCES items (id) ON DELETE CASCADE
);

CREATE INDEX idx_pages_manga_pos ON pages (manga_id, pos);

CREATE TABLE container_order (
    container TEXT    NOT NULL,  -- 'root' 或文件夹 id
    item_id   TEXT    NOT NULL,
    pos       INTEGER NOT NULL,
    PRIMARY KEY (container, item_id)
    -- 刻意不给 item_id 加外键：容器键 'root' 不是条目，且历史上手改过的索引
    -- 可能留下指向不存在条目的顺序项（LibraryIndex::children 读时有兜底）。
    -- 写入侧由 save() 负责过滤这类悬空 id，与读取侧的行为保持一致。
);
