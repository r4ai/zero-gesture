# Frontend Development Guide

## 技術スタック

- React
- Tailwind CSS
- Tailwind Varints & Tailwind Merge
- TanStack Router
- TanStack Query
- React Aria Components
- Sonner

## 開発

Storybook駆動で開発する。

## デザイン

Pencil MCPを使う。
デザイントークンを使い、デザインシステムに従い、統一感を持たせる。

## フォーム

データのQuery, MutateにはTanStack Queryを使う。
バリデーションはvalibotを使い、field-levelで行う。
Suspenseなど、Reactの最新機能を使い、素朴なコードを目指す。

## 状態管理

状態は最小で局所化する。根の方で管理するのは最小限にする。
configはuseConfigDraftで一元管理する。
config queryはdocumentだけでなくEngineが返すrevision/generationを保持する。
save/importは編集開始時に観測したrevisionを送信し、Applied成功時は返された
observationでquery cacheを置換する。revision 0とconfigなしはinvalid startupの
typed recovery状態であり、default draftを保存して修復できる。

## コマンド

npm ではなく pnpm を使う。
