# Clean 页前端架构重构总结

> 范围：`src/pages/Clean/index.tsx`（原 **1067 行**）及其周边组件、常量、跨页复用
> 原则：避免过度设计——**不引入 Zustand/Context/useReducer**；不抽尚未稳定或变化频繁的视图（如顶部状态栏）；不为了组件化而组件化

---

## 背景

`Clean/index.tsx` 在重构前已膨胀至 1067 行，并暴露以下四类问题：

| 维度 | 具体问题 |
|---|---|
| **样式复用** | `CLEAN_THEME_VARS` 在 Clean/Optimize/ScanResult 三处重复定义；4 处按钮硬编码 `width: 206, height: 60`；勾选框样式 `checkboxStyle` 在 Clean 与 Optimize 重复；硬编码颜色（`#3B82F6` 等）散落多处 |
| **结构拆分** | 扫描骨架视图、分组行、子项行、清理底部操作区、错误提示固定占位等异质 UI 全部平铺在 1067 行的 JSX 中 |
| **状态管理** | 12+ 顶层 `useState`、7 个 `useRef`、6 个 `useEffect` 全部平铺在页面级；事件订阅、动画队列、store 持久化、状态机混在一起 |
| **死代码/残留** | `PHASE_TO_GROUP`（-30 行）、`CATEGORY_GROUPS` 的 `icon/color` 死字段、`console.log` 调试残留 |

---

## 重构目标

1. **样式复用**：跨页重复的色板/按钮/勾选框样式统一到 `constants/theme.ts` 与 `components/ui/MoleCheckbox`
2. **结构拆分**：按"视图子区域"边界拆组件，组件职责单一；memo 化高频重渲染行（清理动画阶段）
3. **状态管理**：按"事件驱动状态簇"封装为 hooks（`useScanEngine` / `useCleanEngine` / `useSelectionPersistence`），页面只持有视图机状态
4. **React 最佳实践**：`useCallback`/`useMemo` 收敛到必要处；hooks 返回值 `useMemo` 包装以保引用稳定
5. **避免过度设计**：顶部状态栏（变化频繁）不抽；不引入额外状态管理库；不强行把"将来可能用"的逻辑抽出去

---

## 重构后文件树

```
src/
├── constants/
│   └── theme.ts                              ← 新增（SEMANTIC_COLORS + PAGE_THEME_VARS）
├── components/ui/
│   ├── index.ts                              ← 修改（导出 MoleCheckbox）
│   └── MoleCheckbox.tsx                      ← 新增（三态 checkbox 全局组件）
└── pages/
    ├── Clean/
    │   ├── index.tsx                         ← 重写：1067 → 414 行
    │   ├── clean.constants.tsx               ← 新增（CATEGORY_GROUPS + categoryIconMap + PRIMARY_CTA_STYLE + 类型）
    │   ├── scan-status.ts                    ← 新增（SECTION_ORDER/SECTION_TO_GROUP + 纯函数）
    │   ├── hooks/
    │   │   ├── useScanEngine.ts              ← 新增
    │   │   ├── useCleanEngine.ts             ← 新增
    │   │   └── useSelectionPersistence.ts    ← 新增
    │   └── components/
    │       ├── ScanningView.tsx              ← 新增（扫描骨架视图）
    │       ├── CategoryRow.tsx               ← 新增（分组行 + 子项列表）
    │       ├── CleanItemRow.tsx              ← 新增（memo 化子项行）
    │       ├── CleanFooter.tsx               ← 新增（底部「恢复默认」）
    │       ├── ProgressLine.tsx              ← 新增（进度条/分隔线二合一）
    │       ├── ScanResult.tsx                ← 重写（2 空格缩进 + 引入 PAGE_THEME_VARS）
    │       ├── CleanStatusBar.tsx            ← 既有（本次未改）
    │       └── CleanLoading.tsx              ← 既有（本次未改）
    └── Optimize/
        └── index.tsx                         ← 修改（同步替换为 MoleCheckbox，删除本地 CheckIcon/checkboxStyle）
```

---

## 实施概览（5 步 + 1 验证）

| 步骤 | 内容 | 结果 |
|---|---|---|
| **1. 零风险清理** | 删 `PHASE_TO_GROUP`（-30 行）、`CATEGORY_GROUPS` 的 `icon/color` 死字段、2 处 `console.log`；`ScanResult.tsx` 修缩进为 2 空格并引入 `PAGE_THEME_VARS` | index.tsx：1067 → 1035 行 |
| **2. 常量提取** | 新建 `constants/theme.ts`（`SEMANTIC_COLORS` + `PAGE_THEME_VARS`）、`clean.constants.tsx`、`scan-status.ts`；index 删除本地定义改 import；替换 4 处按钮 `style` 为 `PRIMARY_CTA_STYLE`；替换硬编码颜色为 `SEMANTIC_COLORS` | 三份 `CLEAN_THEME_VARS` 收敛为 1 份 |
| **3. 全局组件** | 新建 `MoleCheckbox`（三态：checked / partial / disabled，支持 partialMark）、`ProgressLine`（`percent + alwaysShow`）；Optimize 同步替换 3 处为 `MoleCheckbox` | Clean/Optimize 共 6 处 checkbox 统一 |
| **4. 页面子组件** | 拆出 `ScanningView` / `CategoryRow`（含 `ChevronIcon`、`AnimatePresence` 清理动画） / `CleanItemRow`（`React.memo`）/ `CleanFooter`；index 主视图替换为 `CategoryRow.map` | index.tsx：→ 620 行 |
| **5. hooks 化** | 按"事件驱动状态簇"封装 `useScanEngine` / `useCleanEngine` / `useSelectionPersistence`；index 页面只保留视图机状态（`phase` / `scanResult` / `scanError` / `doneSummary` / `expandedCategories`） | index.tsx：→ 414 行 |
| **6. tsc 验证** | `npx tsc --noEmit` 全量 | 重构相关文件零错误 |

---

## 关键设计决策

### 1. 不引入额外状态管理库

页面机状态只有 5 个（`phase` / `scanResult` / `scanError` / `doneSummary` / `expandedCategories`），用 `useState` 足够。引入 Zustand/Context 会增加心智负担与跨组件渲染耦合。子组件通过 props + callback 通信即可。

### 2. hooks 按"事件驱动状态簇"封装

| Hook | 状态簇 | 订阅的事件 | 关键 API |
|---|---|---|---|
| `useScanEngine(active)` | `scanTarget` / `scanProgress` / `accumulatedSizeKb` / `scanCompletedSections` | `cleanup::phase-result` | `reset()` / `complete()` |
| `useCleanEngine(active)` | `cleanProgress` / `cleanCurrent` / `cleanedItemKeys` + 动画队列 | `clean::apply-progress` | `prepare()` / `applyClean(queue, scanId)` / `cancelClean()` |
| `useSelectionPersistence()` | `selectedItemIds` + store 偏好 + 后端默认勾选 | — | `resetForNewScan()` / `initializeFromScan()` / `toggleItem` / `toggleGroup` / `resetToDefault` / `hasChangedFromDefault` / `persist` |

- `active` 守门：只在对应阶段订阅事件，避免无效回调
- 返回值用 `useMemo` 包装，回调类 API（`reset` / `applyClean` / `toggleItem` 等）引用稳定
- 错误归一化：`applyClean` 返回 `CleanOutcome { ok, totalCleaned, failedCount, error }`，不再让 `try/catch` 散落在页面级
- 动画定时器与卸载清理封装在 hook 内部，页面无 `cleanTimerRef` / `cleanQueueRef` 等中间状态

### 3. 组件按"视图子区域"拆分

- `ScanningView`：扫描骨架（标题 + 进度条 + 阶段列表 + 取消按钮），自含布局与样式
- `CategoryRow`：分组标题行 + 展开的子项列表（含 `ChevronIcon`、`AnimatePresence` 清理动画）
- `CleanItemRow`：`React.memo` 化，props 全原始值（`item` / `isSelected` / `isCleaning` + 稳定 callback），清理动画阶段仅受影响行重渲染
- `CleanFooter`：仅 review 阶段可见的「恢复默认」按钮
- `ProgressLine`：`percent + alwaysShow`，review/cleaning 阶段自动切换"分隔线"与"进度条"两种形态

### 4. 不抽的视图（克制原则）

- **顶部状态栏**（`CleanLoading` + `CleanStatusBar` + 立即清理按钮）：文案/布局在 idle/review/cleaning 三态间变化频繁，组合在页面级更直接
- **scanning 视图的页面骨架**：阶段列表与勾选列表的差异过大，复用率低
- **优化按钮以外的右侧区域**：暂无重复

### 5. 全局组件边界

- `MoleCheckbox`：`Clean`（4 处，partial 三角）与 `Optimize`（3 处，部分 partialMark 为 "–"）共用
- `PRIMARY_CTA_STYLE`：Clean 4 处主按钮统一 206×60
- `SEMANTIC_COLORS`：8 处硬编码颜色统一（accentBlue / warningYellow / successGreen / dangerRed / cautiousOrangeRed）
- `PAGE_THEME_VARS`：原 `CLEAN_THEME_VARS` 在 Clean/Optimize/ScanResult 三份重复 → 收敛为 1 份

---

## 行为保持说明

重构严格保持对外行为不变，重点对应如下：

| 行为 | 保持方式 |
|---|---|
| 事件订阅时机 | hooks 的 `active` 守门与原 `if (phase !== 'scanning') return` 等价 |
| 扫描进度公式 | `((prev/100)*ESTIMATED_SCAN_PHASES + 1) / ESTIMATED_SCAN_PHASES * 100` 原样迁移 |
| 清理动画 400ms 弹出 + 500ms 收尾 | 完整保留在 `useCleanEngine` |
| 无 `scan_id` 时回退到 `review` | `handleClean` 显式检查并 `setPhase('review')` |
| 错误时 `failedCount: 1` | `CleanOutcome` 在 catch 分支归一化 |
| 「下次要按这次的调整来清理吗」弹窗 | `useSelectionPersistence.hasChangedFromDefault()` + `persist()` 两步显式表达 |
| 自动展开所有分类（review 进入时） | `useEffect [phase]` 保留在页面级 |
| 自动扫描入口（Home 一键跳转） | `autoScanConsumedRef` 守门逻辑保留 |

### 修复的隐藏时序问题

原 `handleClean` 开头 `setCleanProgress(0)` 在弹窗之后执行，导致弹窗判定期间 review 视图的 `ProgressLine` 显示**上次清理残留的 100% 进度条**。本次在 hook 暴露 `prepare()`，由 `handleClean` 开头同步调用，把复位提前到弹窗判定之前。

---

## 行数对比

| 文件 | 重构前 | 重构后 | 变化 |
|---|---:|---:|---:|
| `pages/Clean/index.tsx` | 1067 | 414 | **-653** |
| `pages/Clean/components/ScanResult.tsx` | ~150 | ~95 | 缩进统一 + 引入常量 |
| 新增 3 个 hooks | 0 | 356 | +356 |
| 新增 4 个页面子组件 | 0 | ~310 | +310 |
| 新增 2 个常量/纯函数文件 | 0 | ~180 | +180 |
| 新增 1 个全局组件 | 0 | ~50 | +50 |
| `pages/Optimize/index.tsx`（同步替换） | — | 净 -20 | 复用 MoleCheckbox |

总代码量因组件化略有增加，但**页面复杂度下降 61%**，且每文件职责单一、可独立读懂。

---

## 验证

```bash
$ npx tsc --noEmit
```

| 错误文件 | 错误数 | 与本次重构关系 |
|---|---:|---|
| `components/reactbits/MagicRings.tsx` | 1 | **既有**：`three` 缺类型声明（重构前即存在） |
| `layout/index.tsx` | 3 | **既有**：`CircleButton` / `circleAccent` / `circleBloom` 未使用（重构前即存在） |
| `utils/iconService.ts` | 1 | **既有**：`BATCH_IDLE` 未使用（重构前即存在） |
| **本次重构相关文件** | **0** | — |

---

## 约定

- **页面级状态**：仅保留视图机（`phase` / `scanResult` / `scanError` / `doneSummary` / `expandedCategories`），其余状态必须封装到 hooks
- **hooks 边界**：按"事件驱动状态簇"拆；返回值用 `useMemo` 包装保稳定；callback API 用 `useCallback([])` 优先
- **样式**：跨页复用走 `constants/theme.ts` 或 `components/ui/`；页面内一次性样式保留在组件
- **新增分组/分类**：仅改 `clean.constants.tsx` 的 `CATEGORY_GROUPS` 与 `categoryIconMap`
- **新增扫描阶段映射**：仅改 `scan-status.ts` 的 `SECTION_ORDER` / `SECTION_TO_GROUP`
- **事件名**：统一从 `constants/tauri-events` 导入，禁止字符串硬编码
- **错误处理**：hook 内部用 try/catch 归一化为返回对象（`CleanOutcome`），不在页面 try/catch
