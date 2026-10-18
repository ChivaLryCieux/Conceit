import { useState } from "react";

export default function App() {
  const [ready] = useState(false);

  return (
    <div className="app-shell">
      <header className="app-header">
        <h1 className="app-title">Conceit</h1>
        <span className="app-subtitle">漫画阅读器</span>
      </header>
      <main className="app-empty">
        {ready ? null : <p>脚手架就绪，书架功能开发中……</p>}
      </main>
    </div>
  );
}
