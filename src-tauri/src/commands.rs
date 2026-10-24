//! Tauri 命令层：书库快照、文件夹、重命名、删除、移动、排序与阅读进度。

use crate::library::{
    build_health_report, copy_dir_all, copy_images_into_manga, delete_manga_files,
    dir_total_bytes, extract_zip, make_cover_thumb, now_unix, remove_cover_thumb, same_page_set,
    scan_images, title_from_path, zip_total_bytes, HealthReport, Item, ItemKind, LibraryIndex,
    MangaMeta, INDEX_FILE_NAME, MANGA_DIR_NAME,
};
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

/// 应用共享状态：书库根目录 + 索引
pub struct AppState {
    pub root: PathBuf,
    pub index_path: PathBuf,
    pub index: LibraryIndex,
    /// 高频小改动（阅读进度）只改内存并标脏，由后台线程定期落盘；
    /// 任何走 save() 的写操作都会顺带冲掉该标记
    pending_flush: bool,
    /// 上次索引落盘时间，save_progress 据此做写盘节流
    last_write: Option<Instant>,
}

/// 阅读进度写盘的最小间隔：窗口内只改内存标脏，由后台线程凑满窗口再落盘
const PROGRESS_WRITE_MIN_INTERVAL: Duration = Duration::from_secs(2);

impl AppState {
    pub fn new(root: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&root)
            .map_err(|e| format!("创建书库目录失败 {}: {e}", root.display()))?;
        let index_path = root.join(INDEX_FILE_NAME);
        let index = LibraryIndex::load(&index_path)?;
        Ok(AppState {
            root,
            index_path,
            index,
            pending_flush: false,
            last_write: None,
        })
    }

    fn save(&mut self) -> Result<(), String> {
        let result = self.index.save(&self.index_path);
        if result.is_ok() {
            self.pending_flush = false;
            self.last_write = Some(Instant::now());
        }
        result
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
    Ok(Snapshot {
        root: st.root.to_string_lossy().replace('\\', "/"),
        index: st.index.clone(),
        manga_counts: st.index.manga_counts(),
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
    let id = id.trim();
    if id.is_empty()
        || id.contains(['/', '\\'])
        || id.split('/').any(|c| c == "..")
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("非法目录 id: {}", id));
    }
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
        let id = target.id.trim();
        if id.is_empty()
            || id.contains(['/', '\\'])
            || id.split('/').any(|c| c == "..")
            || !id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!("非法条目 id: {}", target.id));
        }
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
/// 阅读时前端约每 400ms 报一次进度，每次都全量序列化整个索引落盘
/// （书库大了 index.json 有数 MB）对闪存不友好：距上次落盘不足
/// [`PROGRESS_WRITE_MIN_INTERVAL`] 时只改内存并标脏，由后台线程定期
/// 冲刷（[`flush_pending_writes`]）。代价是进程被杀时最多丢最后约
/// 2 秒的进度（原先为 400ms），对阅读位置无感。
#[tauri::command]
pub fn save_progress(
    state: State<'_, Mutex<AppState>>,
    id: String,
    progress: f64,
) -> Result<(), String> {
    let mut st = lock(&state)?;
    let item = st
        .index
        .item_mut(&id)
        .ok_or_else(|| "条目不存在".to_string())?;
    if let Some(meta) = item.manga.as_mut() {
        meta.progress = progress.clamp(0.0, 1.0);
    }
    if st
        .last_write
        .is_some_and(|t| t.elapsed() < PROGRESS_WRITE_MIN_INTERVAL)
    {
        st.pending_flush = true;
        return Ok(());
    }
    st.save()
}

/// 冲刷延迟落盘的阅读进度：lib.rs 的后台线程每秒调用一次。
/// 与 save_progress 用同一个节流窗口——凑满 2 秒、且有脏数据才真正写盘。
pub fn flush_pending_writes(state: &State<'_, Mutex<AppState>>) {
    let mut st = match lock(state) {
        Ok(st) => st,
        Err(_) => return,
    };
    if !st.pending_flush {
        return;
    }
    if st
        .last_write
        .is_some_and(|t| t.elapsed() < PROGRESS_WRITE_MIN_INTERVAL)
    {
        return;
    }
    if let Err(e) = st.save() {
        eprintln!("落盘延迟的阅读进度失败: {e}");
    }
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

/// 由路径推断标题（供导入时使用，前端可直接用）
#[tauri::command]
pub fn suggest_title(path: String) -> Result<String, String> {
    Ok(title_from_path(&PathBuf::from(path)))
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
    // root 先克隆出来：之后 item_mut 可变借用的是 index 字段，与 root 无冲突
    let root = st.root.clone();
    let item = st
        .index
        .item_mut(&id)
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
        meta.thumb = make_cover_thumb(&root, &id, &c);
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
