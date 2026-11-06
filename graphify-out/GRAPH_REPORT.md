# Graph Report - MineRust  (2026-11-07)

## Corpus Check
- 5 files · ~1,930 words
- Verdict: corpus is large enough that graph structure adds value.

## Summary
- 35 nodes · 32 edges · 5 communities
- Extraction: 100% EXTRACTED · 0% INFERRED · 0% AMBIGUOUS
- Token cost: 0 input · 0 output

## Graph Freshness
- Built from commit: `19a149b8`
- Run `git rev-parse HEAD` and compare to check if the graph is stale.
- Run `graphify update .` after code changes (no API cost).

## Community Hubs (Navigation)
- ARCHITECTURE.md — многопоточный Minecraft на Rust
- AGENTS.md — правила для ИИ-агента (opencode)
- PROGRESS.md — прогресс проекта
- MineRust

## God Nodes (most connected - your core abstractions)
1. `ARCHITECTURE.md — многопоточный Minecraft на Rust` - 11 edges
2. `AGENTS.md — правила для ИИ-агента (opencode)` - 7 edges
3. `MineRust` - 7 edges
4. `PROGRESS.md — прогресс проекта` - 4 edges
5. `Что сделано` - 2 edges
6. `Главные правила` - 1 edges
7. `Юридические границы (строго)` - 1 edges
8. `Стиль кода` - 1 edges
9. `Как ориентироваться в проекте (graphify)` - 1 edges
10. `Тесты` - 1 edges

## Surprising Connections (you probably didn't know these)
- None detected - all connections are within the same source files.

## Import Cycles
- None detected.

## Communities (5 total, 0 thin omitted)

### Community 0 - "ARCHITECTURE.md — многопоточный Minecraft на Rust"
Cohesion: 0.18
Nodes (11): 10. Риски, 1. Цель, 2. Принципы, 3. Структура workspace, 4. Модель потоков (ядро проекта), 5. Данные и совместимость, 6. Клиент (после сервера), 7. Лаунчер (+3 more)

### Community 1 - "AGENTS.md — правила для ИИ-агента (opencode)"
Cohesion: 0.25
Nodes (7): AGENTS.md — правила для ИИ-агента (opencode), Главные правила, Если застрял, Как ориентироваться в проекте (graphify), Стиль кода, Тесты, Юридические границы (строго)

### Community 2 - "PROGRESS.md — прогресс проекта"
Cohesion: 0.25
Nodes (5): M0 — Каркас, PROGRESS.md — прогресс проекта, Известные проблемы / блокеры, Что дальше, Что сделано

### Community 3 - "MineRust"
Cohesion: 0.29
Nodes (7): Building, Goals, Layout, Legal, License, MineRust, Status

## Knowledge Gaps
- **25 isolated node(s):** `Главные правила`, `Юридические границы (строго)`, `Стиль кода`, `Как ориентироваться в проекте (graphify)`, `Тесты` (+20 more)
  These have ≤1 connection - possible missing edges or undocumented components.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `ARCHITECTURE.md — многопоточный Minecraft на Rust` connect `ARCHITECTURE.md — многопоточный Minecraft на Rust` to `PROGRESS.md — прогресс проекта`?**
  _High betweenness centrality (0.348) - this node is a cross-community bridge._
- **Why does `MineRust` connect `MineRust` to `PROGRESS.md — прогресс проекта`?**
  _High betweenness centrality (0.230) - this node is a cross-community bridge._
- **What connects `Главные правила`, `Юридические границы (строго)`, `Стиль кода` to the rest of the system?**
  _25 weakly-connected nodes found - possible documentation gaps or missing edges._