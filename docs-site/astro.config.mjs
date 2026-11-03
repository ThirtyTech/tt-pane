import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://thirtytech.github.io',
  base: '/tt-pane',
  integrations: [starlight({
    title: 'tt-pane',
    description: 'A little structure. A lot more flow. Keyboard-first window positioning for your desktop.',
    logo: { src: './src/assets/tt-pane.svg' },
    favicon: '/favicon.svg',
    customCss: ['./src/styles/theme.css'],
    components: { Hero: './src/components/Hero.astro' },
    social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/thirtytech/tt-pane' }],
    sidebar: [
      { label: 'Start here', items: [{ label: 'Welcome', slug: 'index' }, { label: 'Installation', slug: 'install' }] },
      { label: 'Make room', items: [{ label: 'Positions & keybinds', slug: 'keybinds' }, { label: 'Resize & refine', slug: 'resizing' }] },
      { label: 'Reference', items: [{ label: 'Configuration & CLI', slug: 'configuration' }, { label: 'Platforms & troubleshooting', slug: 'platforms' }] },
    ],
  })],
});
