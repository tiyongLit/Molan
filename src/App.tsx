import { useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

interface DirInfo {
  name: string;
  path: string;
  size: number;
}

function App() {
  const [files, setFiles] = useState<DirInfo[]>([]);
  const [loading, setLoading] = useState(false);

  // 格式化文件大小
  const formatSize = (bytes: number) => {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
  };

  const handleScan = async () => {
    setLoading(true);
    try {
      // 1. 让用户选择文件夹 (获取沙箱权限的关键)
      const selected = await open({ directory: true });
      if (selected) {
        // 2. 调用 Rust 后端扫描
        const data = await invoke<DirInfo[]>('scan_folder', { path: selected });
        setFiles(data);
      }
    } catch (error) {
      console.error('Scan failed:', error);
      alert('扫描失败: ' + error);
    } finally {
      setLoading(false);
    }
  };

  const handleClean = async (path: string) => {
    if (confirm(`确定要将 "${path}" 移到废纸篓吗？`)) {
      try {
        await invoke('clean_files', { paths: [path] });
        // 清理成功后从列表中移除
        setFiles(files.filter(f => f.path !== path));
        alert('已移到废纸篓');
      } catch (error) {
        alert('清理失败: ' + error);
      }
    }
  };

  return (
    <div style={{ padding: '20px' }}>
      <h1>Mole Demo - MVP</h1>
      <button onClick={handleScan} disabled={loading}>
        {loading ? '扫描中...' : '选择文件夹并扫描'}
      </button>

      <ul style={{ marginTop: '20px' }}>
        {files.map((file) => (
          <li key={file.path} style={{ marginBottom: '10px', borderBottom: '1px solid #eee', paddingBottom: '5px' }}>
            <strong>{file.name}</strong> - {formatSize(file.size)}
            <button
              onClick={() => handleClean(file.path)}
              style={{ marginLeft: '10px', color: 'red' }}
            >
              清理
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

export default App;
