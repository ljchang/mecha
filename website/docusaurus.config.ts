import {themes as prismThemes} from 'prism-react-renderer';
import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

const config: Config = {
  title: 'mecha',
  tagline: 'A standalone agent harness — extracted so it can be reused, not rewritten',
  favicon: 'img/favicon.ico',

  future: {
    v4: true,
  },

  url: 'https://ljchang.github.io',
  baseUrl: '/mecha/',

  organizationName: 'ljchang',
  projectName: 'mecha',
  trailingSlash: false,

  // A broken link is a docs bug, and CI is the right place to find it.
  onBrokenLinks: 'throw',

  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  markdown: {
    mermaid: true,
    hooks: {
      onBrokenMarkdownLinks: 'throw',
    },
  },
  themes: ['@docusaurus/theme-mermaid'],

  presets: [
    [
      'classic',
      {
        docs: {
          sidebarPath: './sidebars.ts',
          routeBasePath: 'docs',
          editUrl: 'https://github.com/ljchang/mecha/tree/main/website/',
        },
        // No blog: this site documents a tool, and an empty blog is a dead link
        // in the navbar rather than a feature.
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    // No logo or social card: the scaffold ships Docusaurus' own artwork, and
    // shipping someone else's mark as this project's is worse than having none.
    colorMode: {
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: 'mecha',
      items: [
        {
          type: 'docSidebar',
          sidebarId: 'docsSidebar',
          position: 'left',
          label: 'Documentation',
        },
        {
          to: '/docs/changelog',
          label: 'Changelog',
          position: 'left',
        },
        {
          href: 'https://github.com/ljchang/mecha',
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'dark',
      links: [
        {
          title: 'Documentation',
          items: [
            {label: 'Overview', to: '/docs/intro'},
            {label: 'Getting started', to: '/docs/getting-started/installation'},
            {label: 'Configuration', to: '/docs/reference/configuration'},
          ],
        },
        {
          title: 'Concepts',
          items: [
            {label: 'Security model', to: '/docs/features/security'},
            {label: 'Learning', to: '/docs/features/learning'},
            {label: 'Evaluation', to: '/docs/features/evaluation'},
          ],
        },
        {
          title: 'Project',
          items: [
            {label: 'GitHub', href: 'https://github.com/ljchang/mecha'},
            {label: 'Changelog', to: '/docs/changelog'},
            {
              label: 'License (MIT)',
              href: 'https://github.com/ljchang/mecha/blob/main/LICENSE',
            },
          ],
        },
      ],
      copyright: `Copyright © ${new Date().getFullYear()} Luke Chang. MIT licensed. Built with Docusaurus.`,
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
      additionalLanguages: ['rust', 'toml', 'bash', 'json'],
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
