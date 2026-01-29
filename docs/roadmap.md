# Sage Implementation Roadmap

## Overview

This roadmap builds incrementally. Each phase results in something testable.

**Philosophy**: Get something working end-to-end, then iterate.

**Note**: Original Rust implementation was pivoted to Python/Letta for better agent memory management and tooling ecosystem.

---

## Phase 0: Foundation ✅ COMPLETE

**Goal**: Project setup, development environment, basic connectivity

- [x] Architecture documentation
- [x] NixOS flake for development
- [x] Maple proxy connectivity (Kimi K2)
- [x] Basic LLM completion working

---

## Phase 1: Signal Integration ✅ COMPLETE

**Goal**: Text Sage via Signal, get a response

- [x] signal-cli JSON-RPC integration
- [x] Phone registration
- [x] Message send/receive
- [x] Typing indicators (continuous refresh)
- [x] User allowlist security

---

## Phase 2: Memory Integration ✅ COMPLETE

**Goal**: Sage remembers things between messages

- [x] Letta container setup (Podman)
- [x] Agent creation with llm_config/embedding_config
- [x] Core memory blocks (persona, human)
- [x] Memory persistence across restarts
- [x] Sage learns facts about you over time

---

## Phase 2.5: Tools Integration ✅ COMPLETE

**Goal**: Sage can search the web

- [x] Tool registration system with Letta
- [x] Brave Search tool (privacy-respecting)
- [x] Tool secrets passed securely (BRAVE_API_KEY)
- [x] Continuous typing indicator during tool execution

---

## Phase 3: Filesystem Tools ✅ COMPLETE

**Goal**: Sage can read/write files in its workspace

- [x] Client-side execution architecture (Sage executes locally, not in Letta sandbox)
- [x] `read_file(path)` - Read file contents
- [x] `write_file(path, content)` - Write/create file
- [x] `list_directory(path)` - List directory contents
- [x] `delete_file(path)` - Delete file
- [x] `run_command(command, timeout)` - Execute shell commands
- [x] Path sandboxing to ~/.sage/workspace
- [x] Approval flow: Letta requests → Sage executes → returns result

**Note**: Tool descriptions must be kept minimal (<200 chars each) due to vLLM/Kimi K2 bug. See `docs/kimi-k2-vllm-tool-bug.md`.

---

## Phase 4: Scheduling & Reminders

**Goal**: Sage can remind you of things

### Tasks

- [ ] Valkey integration (Redis-compatible)
- [ ] Background scheduler task
- [ ] Reminder tool
  - [ ] `set_reminder(message, time)`
  - [ ] Sends Signal message when due
- [ ] Test: "Remind me to call mom in 2 hours"

### Estimated Time: 1 week

---

## Phase 5: Gmail Integration

**Goal**: Sage can read your email

### Tasks

- [ ] Google OAuth setup
- [ ] Gmail API client
- [ ] Gmail tools
  - [ ] `list_emails(query, limit)`
  - [ ] `read_email(id)`
  - [ ] `search_emails(query)`
- [ ] Test: "How many unread emails do I have?"

### Estimated Time: 1-2 weeks

---

## Phase 6: Sub-Agent System

**Goal**: Sage can delegate complex tasks

### Tasks

- [ ] Sub-agent manager design
- [ ] Spawn concurrent tasks with isolated context
- [ ] Progress monitoring
- [ ] Smart notification logic (short vs long tasks)
- [ ] Test: "Find the email from Bob about the meeting"

### Estimated Time: 2 weeks

---

## Phase 7: Google Calendar

**Goal**: Sage knows your schedule

### Tasks

- [ ] Google Calendar API (reuse OAuth from Phase 5)
- [ ] Calendar tools
  - [ ] `list_events(start, end)`
  - [ ] `create_event(title, time, duration)`
  - [ ] `check_free_time(date)`
- [ ] Test: "What's on my calendar tomorrow?"

### Estimated Time: 1 week

---

## Phase 8: Web Browser

**Goal**: Sage can browse the web

### Tasks

- [ ] Headless browser integration (Playwright/Selenium)
- [ ] Browser tools
  - [ ] `navigate(url)`
  - [ ] `screenshot()`
  - [ ] `read_page_content()`
- [ ] Test: Navigation and content extraction

### Estimated Time: 2 weeks

---

## Phase 9: Autonomy & Confirmation

**Goal**: Sage asks before risky actions

### Tasks

- [ ] Action categorization (read/write/delete/irreversible)
- [ ] Confirmation flow via Signal
- [ ] Autonomy configuration

### Estimated Time: 1 week

---

## Phase 10: Proactive Monitoring

**Goal**: Sage watches things and alerts you

### Tasks

- [ ] Polling framework
- [ ] Email monitoring (important emails)
- [ ] Calendar reminders
- [ ] Daily briefing option

### Estimated Time: 1-2 weeks

---

## Phase 11: MCP Client

**Goal**: Sage connects to external MCP servers

### Tasks

- [ ] MCP client protocol implementation
- [ ] Server discovery and configuration
- [ ] Dynamic tool registration from MCP

### Estimated Time: 2 weeks

---

## Phase 12: Audit & Observability

**Goal**: Full visibility into Sage's actions

### Tasks

- [ ] Structured audit logging
- [ ] PostgreSQL schema for logs
- [ ] Query interface

### Estimated Time: 1 week

---

## Phase 13: Production Deployment

**Goal**: Sage runs reliably 24/7

### Tasks

- [ ] Production container setup
- [ ] Cloudflare Tunnel for remote access
- [ ] Monitoring & alerting
- [ ] Backup strategy

### Estimated Time: 1 week

---

## Timeline Summary

| Phase | Name | Status | Time |
|-------|------|--------|------|
| 0 | Foundation | ✅ Complete | - |
| 1 | Signal Integration | ✅ Complete | - |
| 2 | Memory Integration | ✅ Complete | - |
| 2.5 | Tools (Brave Search) | ✅ Complete | - |
| 3 | Filesystem Tools | ✅ Complete | - |
| 4 | Reminders | 🔜 Next | 1 week |
| 5 | Gmail | Planned | 1-2 weeks |
| 6 | Sub-Agents | Planned | 2 weeks |
| 7 | Calendar | Planned | 1 week |
| 8 | Web Browser | Planned | 2 weeks |
| 9 | Autonomy | Planned | 1 week |
| 10 | Monitoring | Planned | 1-2 weeks |
| 11 | MCP Client | Planned | 2 weeks |
| 12 | Audit | Planned | 1 week |
| 13 | Production | Planned | 1 week |

**Current state**: Functional Sage with memory, web search, and filesystem access
**Next milestone**: Reminders/scheduling (Phase 4)
