//! Tauri 命令层：书库快照、文件夹、重命名、删除、移动、排序与阅读进度。

use crate::db;
use crate::library::{
    build_health_report, build_manga_zip, copy_dir_all, copy_images_into_manga,
    delete_manga_files, dir_total_bytes, extract_zip, make_cover_thumb, now_unix,
    remove_cover_thumb, same_page_set, scan_images, sanitize_zip_name, title_from_path,
    unique_dest_path, zip_total_bytes, HealthReport, Item, ItemKind, LibraryIndex, MangaMeta,
    INDEX_FILE_NAME, MANGA_DIR_NAME,
};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::State;

/// 导入进度：经 Channel 推给前端，驱动忙碌遮罩里的进度条。
///
/// 安卓的导入分两段（SAF 拷进缓存 → Rust 拷进书库），两段都往同一个
/// Channel 里报，字段保持一致，前端无需区分来源。
#[derive(Serialize, Clone)]
pub struct ImportProgress {
    /// 阶段：copy = 复制 / 解压，thumb = 生成封面缩略图
    pub phase: &'static str,
    /// 当前是第几部（从 1 开始）
    pub index: usize,
    /// 一共几部
    pub total: usize,
    /// 当前漫画标题
    pub title: String,
    /// 当前这部已处理的字节数
    pub done_bytes: u64,
    /// 当前这部预计总字节数；0 = 未知（前端走不确定态）
    pub total_bytes: u64,
}

/// 应用共享状态：书库根目录 + 书架索引数据库 + 内存中的索引
pub struct AppState {
    pub root: PathBuf,
    /// 书架索引数据库（`library.db`）：条目的增删改查都落在这里
    pub conn: Connection,
    pub index: LibraryIndex,
}

impl AppState {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&root)
            .map_err(|e| format!("创建书库目录失败 {}: {e}", root.display()))?;
        let conn = db::open(&root.join(db::DB_FILE_NAME))?;
        // 旧版 index.json 一次性迁入数据库（老用户无感升级）。
        // 图片文件自始至终是普通文件，迁移只搬元数据；迁完改名留底，
        // 之后启动就不再读取它
        let json_path = root.join(INDEX_FILE_NAME);
        LibraryIndex::migrate_json_if_present(&conn, &json_path)?;
        let index = LibraryIndex::load(&conn)?;
        Ok(AppState { root, conn, index })
    }

    /// 结构性改动后整表落盘（内存索引是唯一事实来源）
    fn save(&mut self) -> Result<(), String> {
        self.index.save(&self.conn)
    }

    /// 保存阅读进度：内存索引与数据库行同步更新。
    ///
    /// 进度是单行数据，直接 `UPDATE` 落盘即可——原先受制于 index.json
    /// 全量重写，需要「2 秒节流窗口 + 后台线程冲刷」兜着；SQLite 之后
    /// 这笔写入是微秒级，节流机制随之退役，进程被杀最多丢最后一条
    /// （约 400ms），对阅读位置无感。
    ///
    /// 内存索引仍是唯一事实来源：这一笔 UPDATE 只是把同一份值落到对应行。
    /// 写失败（如磁盘满）不影响内存，下一次结构性 save() 会整表纠正。
    fn save_progress(&mut self, id: &str, progress: f64) -> Result<(), String> {
        let value = progress.clamp(0.0, 1.0);
        let item = self
            .index
            .item_mut(id)
            .ok_or_else(|| "条目不存在".to_string())?;
        if let Some(meta) = item.manga.as_mut() {
            meta.progress = value;
        }
        // 条目不存在时影响行数为 0，不算错误（与「非漫画静默跳过」一致）
        self.conn
            .execute(
                "UPDATE items SET progress = ?1 WHERE id = ?2",
                params![value, id],
            )
            .map(|_| ())
            .map_err(|e| format!("保存阅读进度失败: {e}"))
    }
}

/// 返回给前端的完整快照
#[derive(Serialize)]
pub struct Snapshot {
    /// 书库根目录（绝对路径）
    pub root: String,
    pub index: LibraryIndex,
    /// 文件夹 id -> 其中（含子文件夹）的漫画数量，前端直接查表
    pub manga_counts: BTreeMap<String, u32>,
    /// 容器键（ROOT_KEY 或文件夹 id）-> 按 [`LibraryIndex::children`] 算好的
    /// 有序子条目 id。排序与兜底规则只在数据层这一份，前端 childrenOf
    /// 直接查表，不再把同一套规则用 TS 重写一遍
    pub children: BTreeMap<String, Vec<String>>,
}

/// 取书库状态锁。锁中毒（某线程持锁时 panic 过）时恢复守卫而不是报错：
/// 所有写操作都是「先改内存、再整体落盘」，中毒不会让状态不可用，
/// 为一次已兜住的 panic 废掉整个应用不值得。
fn lock<'a>(
    state: &'a State<'_, Mutex<AppState>>,
) -> Result<std::sync::MutexGuard<'a, AppState>, String> {
    state
        .inner()
        .lock()
        .or_else(|poisoned| Ok(poisoned.into_inner()))
}

/// 体检清理与收编孤儿目录共用的 id 白名单校验：id 会直接拼进磁盘路径，
/// 只放行字母数字与 `-` / `_`，挡住目录穿越与奇怪字符
fn sanitize_id(raw: &str) -> Result<&str, String> {
    let id = raw.trim();
    if id.is_empty()
        || id.contains(['/', '\\'])
        || id.split('/').any(|c| c == "..")
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("非法条目 id: {raw}"));
    }
    Ok(id)
}

/// 导入中途失败时清掉半成品目录：解压 / 复制到一半（常见于空间不足）后
/// 若直接返回错误，垃圾目录会留在书库里白占空间。
fn cleanup_partial_import(dest: &Path) {
    if dest.exists() {
        if let Err(e) = std::fs::remove_dir_all(dest) {
            eprintln!(
                "清理半成品导入目录失败 {}: {e}",
                dest.display()
            );
        }
    }
}

/// 读取整个书架
#[tauri::command]
pub fn library_snapshot(state: State<'_, Mutex<AppState>>) -> Result<Snapshot, String> {
    let st = lock(&state)?;
    // 每个容器（root + 每个文件夹）的有序子条目，一次算好随快照下发
    let mut children = BTreeMap::new();
    children.insert(
        crate::library::ROOT_KEY.to_string(),
        st.index
            .children(None)
            .iter()
            .map(|i| i.id.clone())
            .collect(),
    );
    for item in &st.index.items {
        if item.kind == ItemKind::Folder {
            children.insert(
                item.id.clone(),
                st.index
                    .children(Some(&item.id))
                    .iter()
                    .map(|i| i.id.clone())
                    .collect(),
            );
        }
    }
    Ok(Snapshot {
        root: st.root.to_string_lossy().replace('\\', "/"),
        index: st.index.clone(),
        manga_counts: st.index.manga_counts(),
        children,
    })
}

/// 新建文件夹
#[tauri::command]
pub fn create_folder(
    state: State<'_, Mutex<AppState>>,
    name: String,
    parent: Option<String>,
) -> Result<Item, String> {
    let title = name.trim().to_string();
    if title.is_empty() {
        return Err("文件夹名称不能为空".to_string());
    }
    let mut st = lock(&state)?;
    if let Some(p) = &parent {
        match st.index.item(p) {
            Some(item) if item.kind == ItemKind::Folder => {}
            _ => return Err("目标文件夹不存在".to_string()),
        }
    }
    let id = st.index.next_id(ItemKind::Folder);
    let item = Item {
        id: id.clone(),
        kind: ItemKind::Folder,
        title,
        parent: parent.clone(),
        show_cover: false,
        manga: None,
        added_at: crate::library::now_unix(),
    };
    st.index.items.push(item.clone());
    st.index.attach(&id, parent.as_deref());
    st.save()?;
    Ok(item)
}

/// 新建一部空漫画（还没有图片），之后在设置里「＋ 添加图片」逐张补充。
/// parent 为 None 表示根目录。
///
/// 与 import_sources 的区别：不读取任何源文件，只在索引里登记一部
/// pages 为空的漫画，并预建它的目录——首张图片由 add_manga_pages
/// 直接写入，封面也会自动落到第一页。
#[tauri::command]
pub fn create_empty_manga(
    state: State<'_, Mutex<AppState>>,
    name: String,
    parent: Option<String>,
) -> Result<Item, String> {
    let title = name.trim().to_string();
    if title.is_empty() {
        return Err("漫画名称不能为空".to_string());
    }
    let mut st = lock(&state)?;
    if let Some(p) = &parent {
        match st.index.item(p) {
            Some(item) if item.kind == ItemKind::Folder => {}
            _ => return Err("目标文件夹不存在".to_string()),
        }
    }
    let item = st.index.insert_empty_manga(&title, parent.as_deref());
    // 预建漫画目录：与导入的漫画保持同一不变量（每部漫画都有 manga/<id>/），
    // 之后 add_manga_pages 直接把图片写进去
    let dir = st.root.join(MANGA_DIR_NAME).join(&item.id);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        // 目录建不出来：撤销索引里的登记，别留给用户一部加不了图的漫画
        // （消耗掉一个 id 序号无妨，id 本就不要求连续）
        st.index.items.retain(|i| i.id != item.id);
        st.index.detach(&item.id);
        return Err(format!("创建漫画目录失败 {}: {e}", dir.display()));
    }
    st.save()?;
    Ok(item)
}

/// 重命名条目
#[tauri::command]
pub fn rename_item(
    state: State<'_, Mutex<AppState>>,
    id: String,
    title: String,
) -> Result<(), String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("名称不能为空".to_string());
    }
    let mut st = lock(&state)?;
    let item = st
        .index
        .item_mut(&id)
        .ok_or_else(|| "条目不存在".to_string())?;
    item.title = title;
    st.save()
}

/// 删除条目（文件夹会递归删除其中的内容与磁盘文件）
#[tauri::command]
pub fn delete_items(state: State<'_, Mutex<AppState>>, ids: Vec<String>) -> Result<(), String> {
    let mut st = lock(&state)?;
    let mut removed_all: Vec<Item> = Vec::new();
    for id in &ids {
        let removed = st.index.remove_recursive(id);
        removed_all.extend(removed);
    }
    // 删除漫画在磁盘上的目录与封面缩略图
    for item in &removed_all {
        if item.kind == ItemKind::Manga {
            let dir = st.root.join(crate::library::MANGA_DIR_NAME).join(&item.id);
            if dir.exists() {
                std::fs::remove_dir_all(&dir)
                    .map_err(|e| format!("删除漫画目录失败 {}: {e}", dir.display()))?;
            }
            crate::library::remove_cover_thumb(&st.root, &item.id);
        }
    }
    st.save()
}

/// 移动条目到另一个文件夹（None 表示移回根目录）
#[tauri::command]
pub fn move_item(
    state: State<'_, Mutex<AppState>>,
    id: String,
    new_parent: Option<String>,
) -> Result<(), String> {
    let mut st = lock(&state)?;
    let item = st
        .index
        .item(&id)
        .ok_or_else(|| "条目不存在".to_string())?
        .clone();
    if let Some(p) = &new_parent {
        match st.index.item(p) {
            Some(target) if target.kind == ItemKind::Folder => {}
            _ => return Err("目标文件夹不存在".to_string()),
        }
        if item.kind == ItemKind::Folder && st.index.is_descendant_or_self(&id, p) {
            return Err("不能把文件夹移动进它自己内部".to_string());
        }
    }
    st.index.detach(&id);
    st.index.attach(&id, new_parent.as_deref());
    if let Some(it) = st.index.item_mut(&id) {
        it.parent = new_parent;
    }
    st.save()
}

/// 保存某个容器的新排序
#[tauri::command]
pub fn reorder(
    state: State<'_, Mutex<AppState>>,
    parent: Option<String>,
    ordered_ids: Vec<String>,
) -> Result<(), String> {
    let mut st = lock(&state)?;
    st.index.set_order(parent.as_deref(), ordered_ids)?;
    st.save()
}

/// 按磁盘实况重新扫描某部漫画：页数、封面（封面页不见了会顺延）与缩略图。
/// 体检里「索引页数与磁盘不一致」的修复入口。
#[tauri::command]
pub fn rescan_manga_pages(
    state: State<'_, Mutex<AppState>>,
    id: String,
) -> Result<Item, String> {
    let mut st = lock(&state)?;
    let root = st.root.clone();
    let item = st
        .index
        .item_mut(&id)
        .ok_or_else(|| "条目不存在".to_string())?;
    if item.manga.is_none() {
        return Err("该条目不是漫画".to_string());
    }
    crate::library::resync_manga_from_disk(&root, item);
    let updated = item.clone();
    st.save()?;
    Ok(updated)
}

/// 把磁盘上已有的漫画目录收进书架（体检里「孤儿目录」的正向处理）。
///
/// 文件已经在 `manga/<id>/` 里，只是索引没记：这里只补索引、不碰文件，
/// id 沿用目录名，封面取第一页。用户在书架 / 体检里看不到内部目录，
/// 这是把「目录里有但前端没显示」的漫画捞回书架的唯一入口。
#[tauri::command]
pub fn adopt_manga_dir(
    state: State<'_, Mutex<AppState>>,
    id: String,
    title: String,
    parent: Option<String>,
) -> Result<Item, String> {
    // id 直接参与拼路径，白名单与体检清理一致
    let id = sanitize_id(&id)?;
    let title = title.trim();
    if title.is_empty() {
        return Err("漫画名称不能为空".to_string());
    }
    // 空字符串当作根目录（前端传 null 或省略都视为根）
    let parent = parent
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty());
    let mut st = lock(&state)?;
    let root = st.root.clone();
    let item = st
        .index
        .adopt_manga_dir(&root, id, title, parent.as_deref())?;
    st.save()?;
    Ok(item)
}

/// 书库体检：把磁盘实况与索引对比，找出「索引有条目但目录丢了」、
/// 「目录在磁盘上但索引里没有（孤儿）」、「疑似重复导入」与「索引页数
/// 和磁盘不一致」，并列出每个漫画目录里实际有什么。
///
/// 用户看不到应用内部目录，导入中途被杀后留下的对不上的状态只能这样
/// 翻到明面上；配合 `library_health_clean` 清理。
#[tauri::command]
pub fn library_health(state: State<'_, Mutex<AppState>>) -> Result<HealthReport, String> {
    // 取出根目录与索引副本后再扫盘：扫盘要读几千个文件的元数据，
    // 不该一直持有状态锁（否则书架快照请求会被堵住）
    let (root, index) = {
        let st = lock(&state)?;
        (st.root.clone(), st.index.clone())
    };
    Ok(build_health_report(&root, &index))
}

/// 体检清理：按前端的判断移除书架条目与 / 或删除磁盘目录。
///
/// 每条目标自带「要不要连文件一起删」：
/// - 索引里的漫画 → 从书架移除；`delete_files` 时连 `manga/<id>/` 与
///   封面缩略图一起删（与 `delete_items` 的磁盘口径一致）
/// - 索引里没有的孤儿目录 → 只允许 `delete_files`（书架上没有它可移除）
#[derive(Debug, Clone, Deserialize)]
pub struct HealthCleanup {
    pub id: String,
    pub delete_files: bool,
}

#[tauri::command]
pub fn library_health_clean(
    state: State<'_, Mutex<AppState>>,
    targets: Vec<HealthCleanup>,
) -> Result<usize, String> {
    if targets.is_empty() {
        return Ok(0);
    }
    let mut st = lock(&state)?;
    let mut cleaned = 0usize;
    for target in &targets {
        // id 直接参与拼路径，挡住穿越与奇怪字符
        let id = sanitize_id(&target.id)?;
        let dir = st.root.join(MANGA_DIR_NAME).join(id);
        // 体检只针对漫画条目：文件夹不会被报出来，这里也一并挡掉
        let in_index = st
            .index
            .item(id)
            .is_some_and(|i| i.kind == ItemKind::Manga);
        if !in_index && !dir.is_dir() {
            return Err(format!("找不到 {id}：索引与磁盘上都没有"));
        }
        if in_index {
            // 漫画没有子条目，等价于普通移除；顺带清掉排序表里的引用
            st.index.remove_recursive(id);
        } else if !target.delete_files {
            // 孤儿目录在书架上没有条目可移除，只能连文件一起删
            return Err(format!("{id} 只是磁盘上的目录（索引里没有），只能连同文件一起删除"));
        }
        if target.delete_files {
            if dir.is_dir() {
                std::fs::remove_dir_all(&dir)
                    .map_err(|e| format!("删除目录失败 {}: {e}", dir.display()))?;
            }
            remove_cover_thumb(&st.root, id);
        }
        cleaned += 1;
    }
    st.save()?;
    Ok(cleaned)
}

/// 保存阅读进度（0.0 - 1.0）
///
/// 阅读时前端约每 400ms 报一次进度。进度是单行数据，直接
/// `UPDATE items SET progress` 落盘即可——原先受制于 index.json 全量重写，
/// 需要「2 秒节流窗口 + 后台线程冲刷」兜着；SQLite 之后这笔写入是微秒级，
/// 节流机制随之退役，进程被杀最多丢最后一条（约 400ms），对阅读位置无感。
#[tauri::command]
pub fn save_progress(
    state: State<'_, Mutex<AppState>>,
    id: String,
    progress: f64,
) -> Result<(), String> {
    let mut st = lock(&state)?;
    st.save_progress(&id, progress)
}

/// 取一部漫画全部图片的绝对路径（前端经 asset 协议加载）
#[tauri::command]
pub fn manga_pages(state: State<'_, Mutex<AppState>>, id: String) -> Result<Vec<String>, String> {
    let st = lock(&state)?;
    let item = st
        .index
        .item(&id)
        .ok_or_else(|| "条目不存在".to_string())?;
    let meta: &MangaMeta = item
        .manga
        .as_ref()
        .ok_or_else(|| "该条目不是漫画".to_string())?;
    let base = st.root.join(&meta.dir);
    Ok(meta
        .pages
        .iter()
        .map(|p| base.join(p).to_string_lossy().replace('\\', "/"))
        .collect())
}

/// 当前平台标识（前端据此在 Android 上切换原生导入流程）
#[tauri::command]
pub fn app_platform() -> &'static str {
    if cfg!(target_os = "android") {
        "android"
    } else {
        "desktop"
    }
}

/// 更新条目设置：名称 / 是否展示封面 / 选择某一页作为封面
#[tauri::command]
pub fn update_item_settings(
    state: State<'_, Mutex<AppState>>,
    id: String,
    title: Option<String>,
    show_cover: Option<bool>,
    cover: Option<String>,
) -> Result<(), String> {
    let mut st = lock(&state)?;
    apply_item_settings(&mut st, &id, title, show_cover, cover)
}

/// 设置更新的实际逻辑。与 `State` 解耦，便于单测（State 在测试里构造不出来）。
fn apply_item_settings(
    st: &mut AppState,
    id: &str,
    title: Option<String>,
    show_cover: Option<bool>,
    cover: Option<String>,
) -> Result<(), String> {
    // root 先克隆出来：之后 item_mut 可变借用的是 index 字段，与 root 无冲突
    let root = st.root.clone();
    let item = st
        .index
        .item_mut(id)
        .ok_or_else(|| "条目不存在".to_string())?;
    if let Some(t) = title {
        let t = t.trim().to_string();
        if !t.is_empty() {
            item.title = t;
        }
    }
    if let Some(sc) = show_cover {
        item.show_cover = sc;
    }
    if let Some(c) = cover {
        let meta = item
            .manga
            .as_mut()
            .ok_or_else(|| "该条目不是漫画，无法设置封面".to_string())?;
        if !meta.pages.iter().any(|p| p == &c) {
            return Err("封面必须是该漫画的某一页".to_string());
        }
        meta.cover = Some(c.clone());
        // 封面页变了，缩略图要跟着重新生成（生成失败则置空，回退原图）
        meta.thumb = make_cover_thumb(&root, id, &c);
    }
    st.save()
}

/// 为缺失封面缩略图的漫画补生成，返回新生成的数量。
///
/// 供应用启动时在后台线程调用。生成一张缩略图需要解码整张原图，耗时可达数百毫秒，
/// 因此分三段执行，尽量不长时间持有状态锁（否则会阻塞前端的快照请求）。
pub fn backfill_thumbnails(state: &State<'_, Mutex<AppState>>) -> Result<usize, String> {
    // 1. 持锁取出待处理清单，随即释放
    let (root, jobs): (PathBuf, Vec<(String, String)>) = {
        let st = lock(state)?;
        let jobs = st
            .index
            .items
            .iter()
            .filter(|i| i.kind == ItemKind::Manga)
            .filter_map(|i| {
                let meta = i.manga.as_ref()?;
                let cover = meta.cover.clone()?;
                // 已有缩略图且文件仍在 → 跳过
                let has_thumb = meta
                    .thumb
                    .as_ref()
                    .is_some_and(|t| st.root.join(t).exists());
                if has_thumb {
                    None
                } else {
                    Some((i.id.clone(), cover))
                }
            })
            .collect();
        (st.root.clone(), jobs)
    };

    // 2. 不持锁生成
    let mut updates: Vec<(String, String)> = Vec::new();
    for (id, cover) in &jobs {
        if let Some(rel) = make_cover_thumb(&root, id, cover) {
            updates.push((id.clone(), rel));
        }
    }
    let done = updates.len();

    // 3. 一次性写回索引
    if !updates.is_empty() {
        let mut st = lock(state)?;
        for (id, rel) in updates {
            if let Some(meta) = st.index.item_mut(&id).and_then(|i| i.manga.as_mut()) {
                meta.thumb = Some(rel);
            }
        }
        st.save()?;
    }
    Ok(done)
}

/// 从本机路径导入漫画：文件夹会递归复制，zip 会解压，均落入 `library/manga/<id>/`。
/// target_folder 为 None 表示导入到根目录。
///
/// `channel` 用于把复制 / 解压的字节进度实时推给前端（进度条）；
/// 桌面端同样可用，调用方传一个不监听的对象即可。
///
/// 加锁分三段（与 `backfill_thumbnails` 同一模式）：复制 / 解压 / 生成缩略图
/// 可能持续数分钟，全程持锁会堵住 `manga_pages`、`save_progress`、
/// `library_snapshot` 等所有命令——用户边导入边打开漫画就会卡死。因此：
/// 1. 短锁校验目标文件夹、取书库根目录；
/// 2. 每部漫画先短锁分配 id，随后**不持锁**做全部磁盘 IO；
/// 3. 再短锁登记条目并**立即落盘**——多部导入中途失败时，已完成的部
///    不会停留在「内存有、磁盘无」的中间态。
#[tauri::command]
pub fn import_sources(
    state: State<'_, Mutex<AppState>>,
    paths: Vec<String>,
    target_folder: Option<String>,
    channel: Channel,
) -> Result<Vec<Item>, String> {
    let root = {
        let st = lock(&state)?;
        if let Some(p) = &target_folder {
            match st.index.item(p) {
                Some(item) if item.kind == ItemKind::Folder => {}
                _ => return Err("目标文件夹不存在".to_string()),
            }
        }
        st.root.clone()
    };
    let total_items = paths.len();
    eprintln!(
        "[import] import_sources 开始: {} 个路径, 目标文件夹 {:?}",
        paths.len(),
        target_folder
    );
    let mut imported: Vec<Item> = Vec::new();
    for (idx, path_str) in paths.iter().enumerate() {
        let src = PathBuf::from(path_str);
        if !src.exists() {
            eprintln!("[import] 失败: 路径不存在 {path_str}");
            return Err(format!("路径不存在: {path_str}"));
        }
        let is_zip = src.is_file()
            && src
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("zip"))
                .unwrap_or(false);
        if !src.is_dir() && !is_zip {
            eprintln!("[import] 失败: 不支持的来源 {path_str}");
            return Err(format!("仅支持导入文件夹或 zip 压缩包: {path_str}"));
        }

        let title = title_from_path(&src);
        // id 分配需要短暂持锁，保证并发导入不会拿到同一个号
        let id = {
            let mut st = lock(&state)?;
            st.index.next_id(ItemKind::Manga)
        };
        eprintln!(
            "[import] {}/{} 《{}》开始 (id={id}, 源: {path_str})",
            idx + 1,
            total_items,
            title
        );
        let rel_dir = format!("{MANGA_DIR_NAME}/{id}");
        let dest = root.join(&rel_dir);

        // 进度上报：总量先按元数据估出来（0 = 估不出，前端走不确定态），
        // 复制过程中按文件节流到约 10 次/秒，避免小文件多时刷爆 IPC
        let total_bytes = if is_zip {
            zip_total_bytes(&src)
        } else {
            dir_total_bytes(&src)
        };
        let send = |phase: &'static str, done_bytes: u64| {
            let _ = channel.send(InvokeResponseBody::Json(
                serde_json::to_string(&ImportProgress {
                    phase,
                    index: idx + 1,
                    total: total_items,
                    title: title.clone(),
                    done_bytes,
                    total_bytes,
                })
                .unwrap_or_default(),
            ));
        };
        let mut done_bytes = 0u64;
        let mut last_sent = Instant::now();
        let mut on_progress = |n: u64| {
            done_bytes += n;
            if last_sent.elapsed() >= Duration::from_millis(100) {
                last_sent = Instant::now();
                send("copy", done_bytes);
            }
        };

        if is_zip {
            if let Err(e) = extract_zip(&src, &dest, &mut on_progress) {
                eprintln!("[import] {id} 解压失败: {e}");
                cleanup_partial_import(&dest);
                return Err(e);
            }
        } else if let Err(e) = copy_dir_all(&src, &dest, &mut on_progress) {
            eprintln!("[import] {id} 复制失败: {e}");
            cleanup_partial_import(&dest);
            return Err(e);
        }
        // 收尾一定报一次 100%，节流可能把最后一次跳过了
        send("copy", total_bytes.max(done_bytes));

        let pages = scan_images(&dest);
        let cover = pages.first().cloned();
        // 生成封面缩略图可能要点时间（解码原图），单独一个阶段让进度条有话说
        send("thumb", total_bytes.max(done_bytes));
        let thumb = cover.as_deref().and_then(|c| make_cover_thumb(&root, &id, c));
        // 读每页文件头测尺寸（不解码像素），阅读器首屏布局直接用真实宽高比
        let dims = crate::library::measure_page_dims(&dest, &pages);
        let item = Item {
            id: id.clone(),
            kind: ItemKind::Manga,
            title,
            parent: target_folder.clone(),
            show_cover: false,
            manga: Some(MangaMeta {
                dir: rel_dir,
                cover,
                thumb,
                pages,
                dims,
                progress: 0.0,
            }),
            added_at: now_unix(),
        };
        eprintln!(
            "[import] {id} 复制与封面处理完成: {} 页 / {} 字节 / 缩略图 {}",
            item.manga.as_ref().map(|m| m.pages.len()).unwrap_or(0),
            done_bytes,
            if item.manga.as_ref().and_then(|m| m.thumb.as_deref()).is_some() {
                "有"
            } else {
                "无"
            }
        );
        {
            let mut st = lock(&state)?;
            st.index.items.push(item.clone());
            st.index.attach(&id, target_folder.as_deref());
            // 逐部落盘：后面的部失败时，已导入的部不会只在内存里
            st.save()?;
        }
        eprintln!("[import] 索引已落盘: {}《{}》", item.id, item.title);
        imported.push(item);
    }
    eprintln!(
        "[import] 本次导入 {} 部: {}",
        imported.len(),
        imported
            .iter()
            .map(|i| format!("{}《{}》", i.id, i.title))
            .collect::<Vec<_>>()
            .join(", ")
    );
    Ok(imported)
}

/// 给漫画追加图片：复制进书库并排在 `pages` 末尾，返回更新后的条目。
///
/// 新页面刻意不插队——追加后用户在设置里拖动即可归位，
/// 这样不会打乱已经自定义过的顺序。
#[tauri::command]
pub fn add_manga_pages(
    state: State<'_, Mutex<AppState>>,
    id: String,
    paths: Vec<String>,
) -> Result<Item, String> {
    let mut st = lock(&state)?;
    let root = st.root.clone();
    match st.index.item(&id) {
        Some(item) if item.manga.is_some() => {}
        Some(_) => return Err("该条目不是漫画".to_string()),
        None => return Err("条目不存在".to_string()),
    }
    if paths.is_empty() {
        return Err("没有选择要添加的图片".to_string());
    }
    let srcs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let added = copy_images_into_manga(&root, &id, &srcs)?;
    // 新页补上尺寸记录，与 pages 保持同步
    let manga_dir = root.join(MANGA_DIR_NAME).join(&id);
    let added_dims = crate::library::measure_page_dims(&manga_dir, &added);
    let item = st
        .index
        .item_mut(&id)
        .ok_or_else(|| "条目不存在".to_string())?;
    let meta = item
        .manga
        .as_mut()
        .ok_or_else(|| "该条目不是漫画".to_string())?;
    let had_pages = !meta.pages.is_empty();
    meta.pages.extend(added);
    meta.dims.extend(added_dims);
    // 页数曾被删光的漫画，补图时顺手把封面补回来
    if !had_pages && meta.cover.is_none() {
        meta.cover = meta.pages.first().cloned();
        meta.thumb = meta
            .cover
            .as_deref()
            .and_then(|c| make_cover_thumb(&root, &id, c));
    }
    st.save()?;
    st.index
        .item(&id)
        .cloned()
        .ok_or_else(|| "条目不存在".to_string())
}

/// 删除漫画中的若干页：索引记录与磁盘文件一并删除，返回更新后的条目。
///
/// 封面页被删时自动顺延到当前第一页并重建缩略图；至少保留一页
/// （要清空请直接删除整部漫画）。
#[tauri::command]
pub fn delete_manga_pages(
    state: State<'_, Mutex<AppState>>,
    id: String,
    pages: Vec<String>,
) -> Result<Item, String> {
    let mut st = lock(&state)?;
    let root = st.root.clone();
    let item = st
        .index
        .item_mut(&id)
        .ok_or_else(|| "条目不存在".to_string())?;
    let meta = item
        .manga
        .as_mut()
        .ok_or_else(|| "该条目不是漫画".to_string())?;
    if pages.is_empty() {
        return Err("没有选择要删除的页面".to_string());
    }
    let current: HashSet<&str> = meta.pages.iter().map(|s| s.as_str()).collect();
    for p in &pages {
        if !current.contains(p.as_str()) {
            return Err(format!("页面不存在: {p}"));
        }
    }
    if meta.pages.len() <= pages.len() {
        return Err("至少需要保留一页图片".to_string());
    }
    delete_manga_files(&root, &id, &pages);
    let deleted: HashSet<String> = pages.into_iter().collect();
    meta.pages.retain(|p| !deleted.contains(p));
    // 尺寸表同步清理，不留已删页的记录
    meta.dims.retain(|p, _| !deleted.contains(p));
    // 封面页被删 → 顺延到当前第一页，缩略图跟着重建
    if meta.cover.as_ref().is_some_and(|c| deleted.contains(c)) {
        meta.cover = meta.pages.first().cloned();
        meta.thumb = match meta.cover.as_deref() {
            Some(c) => make_cover_thumb(&root, &id, c),
            None => {
                remove_cover_thumb(&root, &id);
                None
            }
        };
    }
    st.save()?;
    st.index
        .item(&id)
        .cloned()
        .ok_or_else(|| "条目不存在".to_string())
}

/// 自定义漫画图片顺序：提交的必须与当前是同一组图片（顺序不同），
/// 返回更新后的条目。集合不一致说明前端状态过期，直接报错让其刷新。
#[tauri::command]
pub fn reorder_manga_pages(
    state: State<'_, Mutex<AppState>>,
    id: String,
    pages: Vec<String>,
) -> Result<Item, String> {
    let mut st = lock(&state)?;
    let item = st
        .index
        .item_mut(&id)
        .ok_or_else(|| "条目不存在".to_string())?;
    let meta = item
        .manga
        .as_mut()
        .ok_or_else(|| "该条目不是漫画".to_string())?;
    if !same_page_set(&meta.pages, &pages) {
        return Err("图片集合与当前不一致，无法保存顺序".to_string());
    }
    meta.pages = pages;
    st.save()?;
    st.index
        .item(&id)
        .cloned()
        .ok_or_else(|| "条目不存在".to_string())
}

/// 导出进度：经 Channel 推给前端。字段名与 Kotlin 侧约定一致，
/// 前端另有一套解析（导出只有「打包」与「落盘」两个阶段）。
#[derive(Serialize, Clone)]
pub struct ExportProgress {
    /// 阶段：zip = 打包，save = 写进下载目录（安卓 MediaStore / SAF 复制）
    pub phase: &'static str,
    /// 已处理的字节数
    pub done_bytes: u64,
    /// 预计总字节数；0 = 未知（前端走不确定态）
    pub total_bytes: u64,
}

/// 把 zip 落进目标目录：同名自动加 " (n)"（不覆盖上一次的导出），
/// 优先 rename（同卷是瞬间完成的元数据操作），跨卷时退回复制再删源。
/// 与 AppHandle 解耦，便于单测。
#[cfg(not(target_os = "android"))]
fn save_into_dir(dir: &Path, zip_path: &Path, file_name: &str) -> Result<String, String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("创建下载目录失败 {}: {e}", dir.display()))?;
    let dest = unique_dest_path(dir, file_name);
    if std::fs::rename(zip_path, &dest).is_err() {
        std::fs::copy(zip_path, &dest)
            .map_err(|e| format!("复制到下载目录失败 {}: {e}", dest.display()))?;
        std::fs::remove_file(zip_path).ok();
    }
    Ok(dest.to_string_lossy().to_string())
}

/// 桌面端：定位系统下载目录（Windows 的「下载」文件夹 / macOS ~/Downloads /
/// Linux 的 XDG_DOWNLOAD_DIR）后把 zip 挪进去
#[cfg(not(target_os = "android"))]
fn save_to_download_dir(
    app: &tauri::AppHandle,
    zip_path: &Path,
    file_name: &str,
) -> Result<String, String> {
    use tauri::Manager;
    let dir = app
        .path()
        .download_dir()
        .map_err(|e| format!("找不到系统的下载目录: {e}"))?;
    save_into_dir(&dir, zip_path, file_name)
}

/// 把一部漫画导出成 zip，落到系统的「下载」目录，返回最终文件路径。
///
/// 桌面端直接把 zip 挪进 `download_dir()`；安卓上书库位于应用私有目录，
/// 前端既读不到也写不出去，由内置 Kotlin 插件经 MediaStore（Android 10+，
/// 无需任何权限）或 SAF「创建文档」选择器（更低版本）写进公共下载目录，
/// 见 [`crate::mobile_bridge::save_to_downloads`]。
///
/// 持锁分两段（与 import_sources 同一模式）：先短锁取出这部漫画的标题 /
/// 目录 / 页表，打包与拷贝全程不持锁——大漫画可能几分钟，期间用户仍能
/// 翻书架、读别的漫画。
#[tauri::command]
pub async fn export_manga(
    app: tauri::AppHandle,
    state: State<'_, Mutex<AppState>>,
    id: String,
    channel: Channel,
) -> Result<String, String> {
    let (title, manga_dir, pages) = {
        let st = lock(&state)?;
        let item = st
            .index
            .item(&id)
            .ok_or_else(|| "条目不存在".to_string())?;
        if item.kind != ItemKind::Manga {
            return Err("文件夹不能导出".to_string());
        }
        let meta = item
            .manga
            .as_ref()
            .ok_or_else(|| "这部漫画还没有图片".to_string())?;
        if meta.pages.is_empty() {
            return Err("这部漫画还没有图片，无可导出".to_string());
        }
        (item.title.clone(), st.root.join(&meta.dir), meta.pages.clone())
    };

    // 进度分母：只 stat 不解码，几百个页也是毫秒级
    let total_bytes: u64 = pages
        .iter()
        .map(|p| {
            std::fs::metadata(manga_dir.join(p))
                .map(|m| m.len())
                .unwrap_or(0)
        })
        .sum();
    let send = |phase: &'static str, done: u64| {
        let _ = channel.send(InvokeResponseBody::Json(
            serde_json::to_string(&ExportProgress {
                phase,
                done_bytes: done,
                total_bytes,
            })
            .unwrap_or_default(),
        ));
    };

    // 先打包进临时目录：安卓上随后由原生桥搬去下载目录，桌面端多是同卷 rename。
    // 目录名带纳秒时间戳——同一秒内连点两次导出也不会撞到同一个 zip 上
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let staging = std::env::temp_dir().join(format!("conceit-export-{stamp}"));
    let file_name = format!("{}.zip", sanitize_zip_name(&title));
    let zip_path = staging.join(&file_name);
    let mut done_bytes = 0u64;
    let mut last_sent = Instant::now();
    let built = build_manga_zip(&manga_dir, &pages, &zip_path, &mut |n| {
        done_bytes += n;
        if last_sent.elapsed() >= Duration::from_millis(100) {
            last_sent = Instant::now();
            send("zip", done_bytes);
        }
    });
    // 收尾一定报一次 100%，节流可能把最后一次跳过了
    if built.is_ok() {
        send("zip", total_bytes.max(done_bytes));
    }
    let entries = match built {
        Ok((_, entries)) => entries,
        Err(e) => {
            std::fs::remove_dir_all(&staging).ok();
            return Err(e);
        }
    };

    let dest = {
        #[cfg(target_os = "android")]
        let result =
            crate::mobile_bridge::save_to_downloads(&app, &zip_path, &file_name, &channel).await;
        #[cfg(not(target_os = "android"))]
        let result = save_to_download_dir(&app, &zip_path, &file_name);
        result
    };
    // 临时目录无论成败都清掉：安卓上 Kotlin 侧已把内容复制走
    std::fs::remove_dir_all(&staging).ok();
    let dest = dest?;
    eprintln!("[export] 《{title}》{entries} 页 -> {dest}");
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "conceit-cmd-{tag}-{}",
            crate::library::now_unix()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 进度走单行 UPDATE 落盘后，重开应用状态（重新 load）应原样读回——
    /// 覆盖「内存有、磁盘无」的中间态不可能残留
    #[test]
    fn progress_survives_app_restart() {
        let root = temp_root("progress");
        let mut state = AppState::new(root.clone()).unwrap();
        let item = state.index.insert_empty_manga("进度测试", None);
        state.save().unwrap();

        state.save_progress(&item.id, 0.37).unwrap();
        drop(state);
        let mut state = AppState::new(root.clone()).unwrap();
        let progress = |s: &AppState| {
            s.index
                .item(&item.id)
                .and_then(|i| i.manga.as_ref())
                .map(|m| m.progress)
        };
        assert_eq!(progress(&state), Some(0.37));

        // 越界值被夹紧；夹紧后的值同样要落到数据库行里
        state.save_progress(&item.id, 1.9).unwrap();
        assert_eq!(progress(&state), Some(1.0));
        drop(state);
        let state = AppState::new(root.clone()).unwrap();
        assert_eq!(progress(&state), Some(1.0));

        std::fs::remove_dir_all(&root).ok();
    }

    /// 换封面必须同时改 cover 与 thumb：thumb 还是旧的那张，书架就会一直
    /// 显示上一任封面（前端 URL 不带变化，WebView 缓存直接命中）。
    /// 这条测试守住 Rust 这一半；前端那一半见 Card.tsx 的 coverUrl 版本号。
    #[test]
    fn update_item_settings_rebuilds_cover_thumb() {
        let root = temp_root("cover-switch");
        let mut state = AppState::new(root.clone()).unwrap();
        let item = state.index.insert_empty_manga("封面切换", None);
        let dir = root.join(MANGA_DIR_NAME).join(&item.id);
        std::fs::create_dir_all(&dir).unwrap();
        // 两张都超过缩略图阈值的大图，否则 make_cover_thumb 会嫌原图够小而返回 None
        for name in ["a.png", "b.png"] {
            let img = image::RgbImage::from_fn(1200, 900, |x, y| {
                image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
            });
            img.save(dir.join(name)).unwrap();
        }
        {
            let meta = state
                .index
                .item_mut(&item.id)
                .unwrap()
                .manga
                .as_mut()
                .unwrap();
            meta.pages = vec!["a.png".into(), "b.png".into()];
            meta.cover = Some("a.png".into());
            meta.thumb = make_cover_thumb(&root, &item.id, "a.png");
        }
        state.save().unwrap();
        let thumb_rel = state
            .index
            .item(&item.id)
            .unwrap()
            .manga
            .as_ref()
            .unwrap()
            .thumb
            .clone()
            .expect("大图应生成缩略图");
        assert_eq!(thumb_rel, format!("{}/{}.jpg", crate::library::THUMB_DIR_NAME, item.id));

        // 换封面：cover 与 thumb 都要跟着走
        apply_item_settings(
            &mut state,
            &item.id,
            None,
            None,
            Some("b.png".to_string()),
        )
        .unwrap();
        {
            let meta = state.index.item(&item.id).unwrap().manga.as_ref().unwrap();            assert_eq!(meta.cover.as_deref(), Some("b.png"));
            assert_eq!(meta.thumb.as_deref(), Some(thumb_rel.as_str()));
        }
        // 落盘后重开也要读回新封面（数据库那半也得对）
        drop(state);
        let state = AppState::new(root.clone()).unwrap();
        let meta = state.index.item(&item.id).unwrap().manga.as_ref().unwrap();
        assert_eq!(meta.cover.as_deref(), Some("b.png"));
        assert_eq!(meta.thumb.as_deref(), Some(thumb_rel.as_str()));

        // 封面不是这一漫画的页：拒绝（state 重新绑定成 mut 只为这一次调用）
        let mut state = AppState::new(root.clone()).unwrap();
        let err =
            apply_item_settings(&mut state, &item.id, None, None, Some("不存在的页.png".to_string()))
                .unwrap_err();
        assert_eq!(err, "封面必须是该漫画的某一页");

        std::fs::remove_dir_all(&root).ok();
    }

    /// 桌面端落盘：zip 挪进目标目录（源消失），同名时加序号、不动旧文件
    #[test]
    #[cfg(not(target_os = "android"))]
    fn save_into_dir_moves_zip_and_never_overwrites() {
        let dir = temp_root("export-dest");
        let staging = temp_root("export-stage");
        let zip = staging.join("book.zip");
        std::fs::write(&zip, b"first").unwrap();

        let dest = save_into_dir(&dir, &zip, "book.zip").unwrap();
        assert_eq!(dest, dir.join("book.zip").to_string_lossy().to_string());
        // rename 走后源文件消失，内容原样落在目标目录
        assert!(!zip.exists());
        assert_eq!(std::fs::read(&dest).unwrap(), b"first");

        // 再导一次同名：写成 book (1).zip，第一次的结果不被覆盖
        std::fs::write(&zip, b"second").unwrap();
        let dest2 = save_into_dir(&dir, &zip, "book.zip").unwrap();
        assert_eq!(dest2, dir.join("book (1).zip").to_string_lossy().to_string());
        assert_eq!(std::fs::read(&dest).unwrap(), b"first");
        assert_eq!(std::fs::read(&dest2).unwrap(), b"second");

        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&staging).ok();
    }

    /// 导出全链路（除平台落盘）：真书库里建一部标题带非法字符的漫画，
    /// 按 export_manga 的同样步骤取信息 → 打包 → 落盘，再解开 zip 校验
    /// 条目名与顺序。这条测试守住「序号条目名 + 阅读顺序」这个契约。
    #[test]
    #[cfg(not(target_os = "android"))]
    fn export_pipeline_numbers_entries_in_reading_order() {
        let root = temp_root("export-e2e");
        let mut state = AppState::new(root.clone()).unwrap();
        let item = state.index.insert_empty_manga("导出/测试:第一卷", None);
        let dir = root.join(MANGA_DIR_NAME).join(&item.id);
        std::fs::create_dir_all(&dir).unwrap();
        // 故意让磁盘上的文件名顺序与阅读顺序不一致：后两页倒着写
        for name in ["c.png", "a.png", "b.png"] {
            std::fs::write(dir.join(name), vec![b'x'; 8]).unwrap();
        }
        {
            let meta = state
                .index
                .item_mut(&item.id)
                .unwrap()
                .manga
                .as_mut()
                .unwrap();
            meta.pages = vec!["a.png".into(), "b.png".into(), "c.png".into()];
            meta.cover = Some("a.png".into());
        }
        state.save().unwrap();

        // —— 以下与 export_manga 的内部步骤一一对应 ——
        let (title, manga_dir, pages) = {
            let found = state.index.item(&item.id).unwrap();
            let meta = found.manga.as_ref().unwrap();
            (found.title.clone(), root.join(&meta.dir), meta.pages.clone())
        };
        assert_eq!(pages, vec!["a.png", "b.png", "c.png"]);
        let staging = temp_root("export-e2e-stage");
        let file_name = format!("{}.zip", sanitize_zip_name(&title));
        // 标题里的 "/" 与 ":" 都被换成下划线
        assert_eq!(file_name, "导出_测试_第一卷.zip");
        let zip_path = staging.join(&file_name);
        let (written, entries) =
            build_manga_zip(&manga_dir, &pages, &zip_path, &mut |_| {}).unwrap();
        assert_eq!(entries, 3);
        assert_eq!(written, 24);

        let dest_dir = temp_root("export-e2e-dest");
        let dest = save_into_dir(&dest_dir, &zip_path, &file_name).unwrap();
        assert!(!zip_path.exists()); // 已挪走

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&dest).unwrap()).unwrap();
        assert_eq!(archive.len(), 3);
        for i in 0..3usize {
            let page = archive.by_index(i).unwrap();
            let expected = format!("{:03}.png", i + 1);
            assert_eq!(page.name(), expected.as_str());
            // 序号严格按阅读顺序：001 = a.png、002 = b.png、003 = c.png
            assert_eq!(page.size(), 8);
        }

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&staging).ok();
        std::fs::remove_dir_all(&dest_dir).ok();
    }

    /// 全新书库：启动即建库，快照与写入都不依赖旧 index.json
    #[test]
    fn fresh_library_starts_empty_and_persists() {
        let root = temp_root("fresh");
        let mut state = AppState::new(root.clone()).unwrap();
        assert!(state.index.items.is_empty());
        assert_eq!(state.index.next_seq, 1);

        let item = state.index.insert_empty_manga("新漫画", None);
        state.save().unwrap();
        drop(state);

        let state = AppState::new(root.clone()).unwrap();
        let loaded = state.index.item(&item.id).expect("重开后条目应在");
        assert_eq!(loaded.title, "新漫画");
        assert_eq!(state.index.next_seq, 2); // 计数器随 meta 落盘

        std::fs::remove_dir_all(&root).ok();
    }
}
