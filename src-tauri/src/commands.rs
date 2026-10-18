//! Tauri 命令层：书库快照、文件夹、重命名、删除、移动、排序与阅读进度。

use crate::library::{
    copy_dir_all, extract_zip, make_cover_thumb, now_unix, scan_images, title_from_path, Item,
    ItemKind, LibraryIndex, MangaMeta, INDEX_FILE_NAME, MANGA_DIR_NAME,
};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

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

fn lock<'a>(
    state: &'a State<'_, Mutex<AppState>>,
) -> Result<std::sync::MutexGuard<'a, AppState>, String> {
    state.inner().lock().map_err(|_| "书库状态锁已中毒".to_string())
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
#[tauri::command]
pub fn import_sources(
    state: State<'_, Mutex<AppState>>,
    paths: Vec<String>,
    target_folder: Option<String>,
) -> Result<Vec<Item>, String> {
    let mut st = lock(&state)?;
    if let Some(p) = &target_folder {
        match st.index.item(p) {
            Some(item) if item.kind == ItemKind::Folder => {}
            _ => return Err("目标文件夹不存在".to_string()),
        }
    }
    let mut imported: Vec<Item> = Vec::new();
    for path_str in &paths {
        let src = PathBuf::from(path_str);
        if !src.exists() {
            return Err(format!("路径不存在: {path_str}"));
        }
        let is_zip = src.is_file()
            && src
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.eq_ignore_ascii_case("zip"))
                .unwrap_or(false);
        if !src.is_dir() && !is_zip {
            return Err(format!("仅支持导入文件夹或 zip 压缩包: {path_str}"));
        }

        let title = title_from_path(&src);
        let id = st.index.next_id(ItemKind::Manga);
        let rel_dir = format!("{MANGA_DIR_NAME}/{id}");
        let dest = st.root.join(&rel_dir);

        if is_zip {
            extract_zip(&src, &dest)?;
        } else {
            copy_dir_all(&src, &dest)?;
        }

        let pages = scan_images(&dest);
        let cover = pages.first().cloned();
        // 导入即为封面生成缩略图（书架只显示一百多像素，没必要让 WebView 解码原图）
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
        st.index.items.push(item.clone());
        st.index.attach(&id, target_folder.as_deref());
        imported.push(item);
    }
    st.save()?;
    Ok(imported)
}
