/// <reference types="vite/client" />
import {
  HeadContent,
  Outlet,
  Scripts,
  createRootRouteWithContext,
} from '@tanstack/react-router'
import type { QueryClient } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import appCss from '@/styles.css?url'

const SITE_URL = 'https://waku.sh'
const TITLE = 'Waku — one native app for all your coding agents'
const DESCRIPTION =
  'A fast, native app for local coding agents. Amp, Claude Code, Codex, Cursor, OpenCode, Grok, and Pi — one timeline, entirely on your machine.'

export const Route = createRootRouteWithContext<{
  queryClient: QueryClient
}>()({
  head: () => ({
    meta: [
      { charSet: 'utf-8' },
      { name: 'viewport', content: 'width=device-width, initial-scale=1' },
      { title: TITLE },
      { name: 'description', content: DESCRIPTION },
      { property: 'og:title', content: TITLE },
      { property: 'og:description', content: DESCRIPTION },
      { property: 'og:type', content: 'website' },
      { property: 'og:url', content: SITE_URL },
      { property: 'og:image', content: `${SITE_URL}/og-icon.png` },
      { name: 'twitter:card', content: 'summary' },
      {
        name: 'theme-color',
        media: '(prefers-color-scheme: light)',
        content: '#ffffff',
      },
      {
        name: 'theme-color',
        media: '(prefers-color-scheme: dark)',
        content: '#1e1e1e',
      },
    ],
    links: [
      { rel: 'stylesheet', href: appCss },
      { rel: 'icon', type: 'image/png', sizes: '32x32', href: '/favicon.png' },
      { rel: 'apple-touch-icon', sizes: '180x180', href: '/apple-touch-icon.png' },
    ],
    scripts: [
      {
        // Mirror the system color scheme onto <html> before first paint.
        children: `try{var m=matchMedia('(prefers-color-scheme: dark)'),d=document.documentElement,s=function(){d.classList.toggle('dark',m.matches)};s();m.addEventListener('change',s)}catch(e){}`,
      },
      // Analytics, production builds only.
      ...(import.meta.env.PROD
        ? [
            {
              defer: true,
              src: 'https://u.egoist.dev/script.js',
              'data-website-id': '5dc2da71-cd6e-4862-8d60-e1cfb782f54f',
            },
          ]
        : []),
    ],
  }),
  component: RootComponent,
})

function RootComponent() {
  return (
    <RootDocument>
      <Outlet />
    </RootDocument>
  )
}

function RootDocument({ children }: { children: ReactNode }) {
  return (
    <html lang="en" suppressHydrationWarning>
      <head>
        <HeadContent />
      </head>
      <body>
        {children}
        <Scripts />
      </body>
    </html>
  )
}
