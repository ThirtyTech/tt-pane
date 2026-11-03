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
    head: [
      { tag: 'meta', attrs: { property: 'og:image', content: 'https://thirtytech.github.io/tt-pane/media/tt-pane-social.png' } },
      { tag: 'meta', attrs: { property: 'og:image:type', content: 'image/png' } },
      { tag: 'meta', attrs: { property: 'og:image:width', content: '1200' } },
      { tag: 'meta', attrs: { property: 'og:image:height', content: '630' } },
      { tag: 'meta', attrs: { property: 'og:image:alt', content: 'tt-pane — Make room for your flow. Small colored windows snap into an organized desktop layout.' } },
      { tag: 'meta', attrs: { name: 'twitter:image', content: 'https://thirtytech.github.io/tt-pane/media/tt-pane-social.png' } },
      { tag: 'meta', attrs: { name: 'twitter:image:alt', content: 'tt-pane — Make room for your flow. Small colored windows snap into an organized desktop layout.' } },
    ],
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
