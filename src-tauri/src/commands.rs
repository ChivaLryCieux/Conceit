//! Tauri 命令层：书库快照、文件夹、重命名、删除、移动、排序与阅读进度。

use crate::library::{
    copy_dir_all, copy_images_into_manga, delete_manga_files, dir_total_bytes, extract_zip,
    make_cover_thumb, now_unix, remove_cover_thumb, same_page_set, scan_images, title_from_path,
    zip_total_bytes, Item, ItemKind, LibraryIndex, MangaMeta, INDEX_FILE_NAME, MANGA_DIR_NAME,
};
use serde::Serialize;
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
}

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
        })
    }

    fn save(&self) -> Result<(), String> {
        self.index.save(&self.index_path)
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

/// 保存阅读进度（0.0 - 1.0）
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
    st.save()
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
    // 先取出根目录，避免与下面的可变借用冲突
    let root = lock(&state)?.root.clone();
    let mut st = lock(&state)?;
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
#[tauri::command]
pub fn import_sources(
    state: State<'_, Mutex<AppState>>,
    paths: Vec<String>,
    target_folder: Option<String>,
    channel: Channel,
) -> Result<Vec<Item>, String> {
    let mut st = lock(&state)?;
    if let Some(p) = &target_folder {
        match st.index.item(p) {
            Some(item) if item.kind == ItemKind::Folder => {}
            _ => return Err("目标文件夹不存在".to_string()),
        }
    }
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
        let id = st.index.next_id(ItemKind::Manga);
        eprintln!(
            "[import] {}/{} 《{}》开始 (id={id}, 源: {path_str})",
            idx + 1,
            total_items,
            title
        );
        let rel_dir = format!("{MANGA_DIR_NAME}/{id}");
        let dest = st.root.join(&rel_dir);

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
        let thumb = cover
            .as_deref()
            .and_then(|c| make_cover_thumb(&st.root, &id, c));
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
        st.index.items.push(item.clone());
        st.index.attach(&id, target_folder.as_deref());
        imported.push(item);
    }
    st.save()?;
    eprintln!(
        "[import] 索引已落盘, 本次导入 {} 部: {}",
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
    let root = lock(&state)?.root.clone();
    let mut st = lock(&state)?;
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
    let root = lock(&state)?.root.clone();
    let mut st = lock(&state)?;
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
    let deleted: HashSet<String> = pages.into_iter().collect();
    let deleted_list: Vec<String> = deleted.iter().cloned().collect();
    delete_manga_files(&root, &id, &deleted_list);
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
