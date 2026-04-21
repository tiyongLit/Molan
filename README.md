# Tauri + React + Typescript

This template should help get you started developing with Tauri, React and Typescript in Vite.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)




2. “对标 Mole 清理效果” vs “上架商店” 的矛盾破解
你担心的红线是：如果为了上架把功能砍没了，那就不做了。

我的方案是：我们不砍功能逻辑，只改触发方式。

Mole 的功能	传统做法 (会被拒)	我们的 MAS 做法 (能过审且效果一样)
全盘扫描	后台静默扫描 /	一键快捷扫描：界面上放“扫描用户目录”、“扫描缓存”按钮，点一下即扫。
强力卸载	自动删除 /Applications	智能残留清理：扫描 ~/Library 下的孤儿文件，用户勾选后移入废纸篓。
系统优化	执行 sudo 命令重置网络	深度建议模式：告诉用户“发现 500MB 无效日志”，提供“在 Finder 中打开”或“移到废纸篓”。
大文件清理	直接 rm -rf	批量移到废纸篓：利用 Rust 极速列出 Top 100 大文件，用户全选后一键 Trash。


心逻辑：

Mole 的效果 = 找到垃圾 + 删掉垃圾。
MAS 版的效果 = 找到垃圾 + 让用户确认 + 移到废纸篓。
结果：垃圾都进了废纸篓，用户清空废纸篓后，清理效果是一模一样的！
