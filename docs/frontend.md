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

## コマンド

npm ではなく pnpm を使う。
