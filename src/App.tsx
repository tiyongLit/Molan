import { RouterProvider } from 'react-router-dom';
import { router } from './routers'
import { MoleMessageProvider } from '@/components/ui'

function App() {
  return (
    <>
      {/* 全局 antd message 挂载点：授权失败等场景的 toast 提示（moleMessage） */}
      <MoleMessageProvider>
        <RouterProvider router={router} />
      </MoleMessageProvider>
    </>
  );
}

export default App;
