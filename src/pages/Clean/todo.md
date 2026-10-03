toolName: view_files

status: success


filePath: /Users/liuy/mole_desktop/mvp/Molan2/src/pages/Clean/index.tsx

分析结果很直接：

### 对比 lemon-cleaner 的差异

**lemon-cleaner 的做法：**
- `ResultCellView.setCellData:` 调用 `[self.iconView setImage:[item iconImage]]`
- `iconImage` 来自扫描器，扫描器**不仅知道文件路径，还知道该文件属于哪个 app**，因此可以直接拿 app 的 bundle 路径（如 `/Applications/Visual Studio Code.app`）去获取 VS Code 的图标

**我们当前的做法：**
- 直接用 `item.path`（即缓存/数据文件夹路径）去 `mole_get_icons_batch` 要图标
- 对于 `/Users/me/Library/Application Support/Code/Cache`，macOS 只会返回一个**普通文件夹图标**，不是 VS Code 图标

### 结论

这条差异的根本原因不在前端，而在于**数据结构**。lemon-cleaner 的扫描器会给每个 item 附带一个 app 标识（用于获取 app 图标），我们目前的 mock 数据没有这个字段。一旦后端扫描器就绪并提供了类似 `item.appBundlePath` 的字段，前端只需改一行：

```tsx
// 当前：拿文件夹图标
iconService.getCachedSync(item.path)

// 将来：拿 app 图标（如果扫描器提供了）
iconService.getCachedSync(item.appBundlePath || item.path)
```

**当前前端代码不需要改动。** 它已经正确实现了"传入路径 → 获取 macOS 原生图标"的逻辑。只是由于 mock 数据的路径在本地磁盘上不存在（`/Users/me/...`），图标始终为 null。等真实扫描数据接入后，图标就会显示了。


# 真实扫描数据接入注意事项

## 1. 原生图标（iconService）

### 当前问题
- mock 数据路径 `/Users/me/...` 在本地不存在，`iconService.getIcon()` 始终返回 `null`
- 即使路径存在，获取的也只是**文件夹图标**，不是关联 app 的图标

### 后端需要提供的字段

```typescript
interface CleanItem {
  // 已有字段
  id: string
  path: string          // 文件/文件夹的实际路径

  // ⚠️ 需要新增的字段（lemon-cleaner 有提供）
  appBundlePath?: string // 关联应用的 .app bundle 路径
                         // 例如 /Applications/Visual Studio Code.app
                         // 前端据此获取原生的 VS Code 图标
}
```

### 前端代码位置

[Clean/index.tsx#L276-L289](file:///Users/liuy/mole_desktop/mvp/Molan2/src/pages/Clean/index.tsx#L276-L289)

```typescript
// 改动点：预加载图标时优先取 appBundlePath，其次取 path
iconService.preloadIconsIdle(paths)  // paths 应包括 appBundlePath
```

---

## 2. 分类图标颜色与 icon 映射

### 当前设计

`CATEGORY_GROUPS` 使用 `@ant-design/icons` 的通用图标（数据库、应用、地球、代码等），所有图标白色。

### 后端需要提供或对齐的

- 确认分类 ID 体系（`system_caches`, `user_cache`, `app_caches`, `browser_cache` 等）与后端扫描器保持一致
- 如果后端扫描器会动态新增分类，则 `CATEGORY_GROUPS` 需要改为运行时生成

### 前端代码位置

[Clean/index.tsx#L155-L168](file:///Users/liuy/mole_desktop/mvp/Molan2/src/pages/Clean/index.tsx#L155-L168)

---

## 3. 推荐/谨慎清理标识

### 当前逻辑

每个 item 有 `recommend` 和 `cautious` 布尔字段：
- `recommend: true` → 显示"建议清理"
- `cautious: true` → 显示"谨慎清理"（橙红色 `#E6704C`）
- 两者都 false / size 为 0 → 显示"很干净"（绿色 `#33D39D`）

### 后端需要注意

- `recommend` 和 `cautious` 不应同时为 true
- size 为 0 时应无条件显示"很干净"，禁用 checkbox
- 前端代码位置：[Clean/index.tsx#L647-L658](file:///Users/liuy/mole_desktop/mvp/Molan2/src/pages/Clean/index.tsx#L647-L658)

---

## 4. 全选/取消全选逻辑

### 当前设计

前一次重构已恢复分组级 checkbox，点击可批量选中/取消该组所有 item。

### 后端注意事项

当用户选择触发"清理"后，后端应接收以下格式的选中项：

```typescript
// 前端发送的清理请求格式
{
  // 1. 按分类分组
  categories: {
    [categoryId: string]: {
      selected_items: string[]  // item.id 数组
    }
  }
  // 2. 或直接传 flat IDs
  selected_ids: string[]  // 格式: "categoryId::itemId"
}
```

两种格式都可以，前端可以适配。

前端代码位置：[Clean/index.tsx#L330-L341](file:///Users/liuy/mole_desktop/mvp/Molan2/src/pages/Clean/index.tsx#L330-L341)

---

## 5. 状态文本风格

### lemon-cleaner 的规则参考

| 条件 | 显示文本 | 颜色 |
|------|---------|------|
| size === 0 | "很干净" | `#33D39D` 绿 |
| recommend && !cautious | "共 X，建议清理" | `text-white/65` 灰 |
| cautious && !recommend | "共 X，谨慎清理" | `#E6704C` 橙红 |

### 后端对齐

确保扫描器产出的 `recommend` / `cautious` 语义与 lemon-cleaner 一致：
- **recommend**: 放心理清理，如系统缓存、日志、浏览器缓存
- **cautious**: 需要谨慎，包含用户数据（如下载文件夹、Xcode DerivedData）

---

## 6. whitelist_matched 标记

### 当前 UI 表现

命中白名单的 item 显示琥珀色 "默认跳过" 标签，样式参考 lemon-cleaner 的温馨提示。

### 后端对齐

扫描数据中的 `whitelist_matched` 字段应与白名单引擎联动。

前端代码位置：[Clean/index.tsx#L634-L639](file:///Users/liuy/mole_desktop/mvp/Molan2/src/pages/Clean/index.tsx#L634-L639)

---

## 7. 清理进度

### 当前模拟

- 2 秒内 0 → 100，无真实后端交互
- 清理完成后显示 `cleanSummary`（totalCleaned, failedCount）

### 后端需要提供

```typescript
// 清理中的进度推送（Tauri event）
{
  type: 'clean_progress'
  percent: number       // 0~100
  currentItem?: string  // 正在清理的路径，用于 UI 显示
}

// 清理完成结果
{
  total_cleaned: number      // bytes
  total_cleaned_human: string
  failed_count: number
  failed_items?: { path: string; reason: string }[]
}
```

### 前端代码位置

[Clean/index.tsx#L343-L360](file:///Users/liuy/mole_desktop/mvp/Molan2/src/pages/Clean/index.tsx#L343-L360)

---

## 8. 其他注意事项

| 项目 | 说明 |
|------|------|
| **权限** | 部分路径需要 `sudo`（`requires_sudo`），清理前应提示或自动提权 |
| **Time Machine 本地快照** | 清理 `.MobileBackups` 有特殊要求，后端需额外处理 |
| **文件计数精度** | `file_count` 在数据量大时可能影响 Preload 速度，建议后端分批返回 |
| **分类稳定性** | 避免在清理过程中动态增删分类，否则 `groups` 和 `selectedItemIds` 会不同步 |
| **路径显示** | 前端会做 `/Users/{username}` → `~` 替换，确保后端返回的是**完整绝对路径** |
