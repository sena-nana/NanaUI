<template>
  <section id="hero">
    <h1 class="tagline">
      <span class="accent">保留式</span>
      <br />原生桌面界面
    </h1>
    <p class="description">易上手的桌面界面框架。一棵界面树，窗口和 GPU 由你的应用持有。</p>
    <p class="actions">
      <a class="get-started" href="/guide/introduction">
        开始
        <svg class="icon" xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 24 24">
          <path d="M13.025 1l-2.847 2.828 6.176 6.176h-16.354v3.992h16.354l-6.176 6.176 2.847 2.828 10.975-11z" />
        </svg>
      </a>
      <a class="setup" href="/guide/quick-start">快速开始</a>
    </p>
  </section>

  <section id="highlights">
    <div class="highlight">
      <h2>一棵树</h2>
      <p>按钮、文字、侧栏和实时画面是同一棵树上的节点。它们一起排版、裁剪，也按同一个顺序接收点击。</p>
    </div>
    <div class="highlight">
      <h2>两种写法</h2>
      <p><code>view!</code> 和函数调用展开成同一套调用。侧边栏顶部的开关切换文档里的示例，运行时还是同一次挂载。</p>
    </div>
    <div class="highlight">
      <h2>画面在树上</h2>
      <p>着色器和预览视口是普通节点。你的应用持有窗口和设备，框架把这棵树画进那块 Surface。</p>
    </div>
  </section>
</template>

<style scoped>
#hero {
  padding: 88px 32px 28px;
  text-align: center;
}

.tagline {
  font-size: 76px;
  line-height: 1.15;
  font-weight: 900;
  letter-spacing: -1.5px;
  max-width: 960px;
  margin: 0 auto;
  border: 0;
  padding: 0;
}

.accent {
  background: linear-gradient(315deg, #7bb9f0 20%, #2c7ed6);
  background-clip: text;
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
}

.description {
  max-width: 720px;
  line-height: 1.6;
  color: var(--vp-c-text-2);
  font-size: 22px;
  margin: 24px auto 36px;
}

.actions {
  margin: 0;
}

.actions a {
  font-size: 16px;
  display: inline-block;
  padding: 8px 18px;
  font-weight: 500;
  border-radius: 8px;
  text-decoration: none;
  transition: background-color 0.2s, color 0.2s;
}

.actions .get-started {
  margin-right: 16px;
  background: var(--vp-c-brand-1);
  color: var(--vp-c-brand-text, #fff);
}

.dark .actions .get-started {
  color: #0d1622;
}

.actions .setup {
  background: var(--vp-c-bg-soft);
  color: var(--vp-c-text-1);
}

.actions .get-started:hover {
  background: var(--vp-c-brand-2);
}

.actions .setup:hover {
  background: var(--vp-c-bg-mute, var(--vp-c-bg-elv));
}

.icon {
  display: inline;
  position: relative;
  top: -1px;
  margin-left: 4px;
  fill: currentColor;
  transition: transform 0.2s;
}

.get-started:hover .icon {
  transform: translateX(2px);
}

#highlights {
  max-width: 960px;
  margin: 28px auto 0;
  padding: 12px 32px 48px;
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 18px;
}

.highlight h2 {
  margin: 0 0 10px;
  padding: 0;
  border: 0;
  font-size: 20px;
  font-weight: 600;
  letter-spacing: -0.3px;
  line-height: 1.3;
}

.highlight h2::first-letter {
  color: var(--vp-c-brand-1);
}

.highlight p {
  margin: 0;
  color: var(--vp-c-text-2);
  font-size: 15px;
  line-height: 1.7;
}

.highlight :deep(code) {
  font-size: 0.92em;
}

@media (max-width: 960px) {
  .tagline {
    font-size: 56px;
  }
  .description {
    font-size: 18px;
  }
  #highlights {
    grid-template-columns: 1fr;
    padding-top: 8px;
  }
}

@media (max-width: 576px) {
  #hero {
    padding: 48px 20px 12px;
  }
  .tagline {
    font-size: 40px;
    letter-spacing: -0.8px;
  }
  .description {
    font-size: 16px;
    margin-bottom: 28px;
  }
  .actions a {
    display: block;
    margin: 10px auto;
    max-width: 220px;
  }
  .actions .get-started {
    margin-right: auto;
  }
}
</style>
