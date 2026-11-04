-- Checks the Files pair's quick-search plugin against stand-ins for Yazi, Zoxide
-- and a slow fzf (EON-C18): a second invocation while one is up returns at once
-- instead of queueing a second fzf, every exit frees the plugin for the next
-- one, fzf's socket is removed when it ends, fzf keeps its last frame, and a
-- pick or Esc is reported while Yazi is still hidden, so Yazi does not redraw
-- Files before the pane. Whether Venus shows a frame is for the installed proof.
-- Usage: lua quick-search-guard.lua PLUGIN SOCKET
local plugin, socket = arg[1], arg[2]

local state, spawned, notes, reports = {}, 0, {}, {}
local hidden = false
local fzf_options -- FZF_DEFAULT_OPTS of the last fzf
local report_fails = false
local fzf_wait -- how the next fzf ends
local zoxide_fails = false

ya = {
  sync = function(f) return function(...) return f(state, ...) end end,
  notify = function(note) notes[#notes + 1] = note.content end,
  emit = function(name, args)
    reports[#reports + 1] = { name, args[1], hidden }
  end,
}
ui = {
  hide = function()
    hidden = true
    return { drop = function() hidden = false end }
  end,
}
package.loaded["tab-folder"] = {
  set = function(dir)
    if report_fails then error("eon vanished") end
    reports[#reports + 1] = { "set", dir, hidden }
  end,
  close = function() reports[#reports + 1] = { "close", nil, hidden } end,
}
cx = { active = { current = { cwd = "/" } } }

local builder = {}
builder.__index = builder
for _, method in ipairs({ "arg", "stdin", "stdout", "stderr" }) do
  builder[method] = function(self) return self end
end
function builder:env(name, value)
  if self.name == "fzf" and name == "FZF_DEFAULT_OPTS" then fzf_options = value end
  return self
end
function builder:spawn()
  if self.name == "zoxide" then
    if zoxide_fails then return nil, "no zoxide" end
    return {
      take_stdout = function() return {} end,
      try_wait = function() return nil end,
      start_kill = function() end,
      wait = function() end,
    }
  end
  spawned = spawned + 1
  local wait = fzf_wait
  return {
    -- A slow fzf: it starts listening, and ends only when resumed.
    wait_with_output = function()
      io.open(socket, "w"):close()
      coroutine.yield("fzf up")
      return wait()
    end,
  }
end
Command = setmetatable({ PIPED = 1, NULL = 2 }, {
  __call = function(_, name) return setmetatable({ name = name }, builder) end,
})

local getenv = os.getenv
os.getenv = function(name)
  if name == "EON_PAIR_SOCKET" then return socket end
  if name == "EON_FILES_NOTICE" then return "" end
  return getenv(name)
end

local M = dofile(plugin)
local function invocation() return coroutine.create(function() M:entry() end) end
local function check(condition, message)
  if not condition then
    io.stderr:write("FAIL: " .. message .. "\n")
    os.exit(1)
  end
end
local function exists(path)
  local file = io.open(path)
  if file then file:close() end
  return file ~= nil
end

-- The runtime's switch to Files ends fzf with status 11.
fzf_wait = function() return { status = { code = 11, success = false }, stdout = "", stderr = "" } end
local first = invocation()
check(select(2, coroutine.resume(first)) == "fzf up", "the first quick search starts fzf")
check(fzf_options and fzf_options:find("--no-clear", 1, true), "fzf keeps its last frame when it ends")
-- Alt+Z again before the first is done.
local second = invocation()
check(coroutine.resume(second), "the second invocation runs")
check(coroutine.status(second) == "dead", "the second invocation returns at once")
check(spawned == 1, "the second invocation starts no second fzf, started " .. spawned)
check(coroutine.resume(first) and coroutine.status(first) == "dead", "the first ends")
check(not exists(socket), "fzf's socket is removed when it ends")

-- Each exit frees the plugin: a failed Zoxide query...
zoxide_fails = true
check(coroutine.resume(invocation()), "a failing invocation runs")
zoxide_fails = false
check(#notes == 1, "the failed query is reported")
-- ...and an error raised while fzf runs.
fzf_wait = function() error("fzf vanished") end
local failing = invocation()
coroutine.resume(failing)
check(coroutine.resume(failing) and coroutine.status(failing) == "dead", "a raised error ends it")
check(#notes == 2, "the raised error is reported")
check(spawned == 2, "the plugin was free after the failures")

-- Then a quick search starts again.
fzf_wait = function() return { status = { code = 1, success = false }, stdout = "", stderr = "" } end
local again = invocation()
check(select(2, coroutine.resume(again)) == "fzf up", "a later quick search starts")
check(spawned == 3, "a later quick search starts its own fzf")
coroutine.resume(again)

-- How each fzf exit is reported, and whether Yazi was still hidden then.
local function ends(code, stdout)
  fzf_wait = function()
    return { status = { code = code, success = code == 0 }, stdout = stdout, stderr = "" }
  end
  reports = {}
  local run = invocation()
  coroutine.resume(run)
  coroutine.resume(run)
  check(coroutine.status(run) == "dead", "quick search ends after status " .. code)
  check(not hidden, "Yazi shows again after status " .. code)
  return reports[1], #reports
end
local report, count = ends(0, "\n/picked\n")
check(count == 1 and report[1] == "set" and report[2] == "/picked", "a pick sets the tab folder")
check(report[3], "a pick is reported while Yazi is hidden")
report, count = ends(130, "")
check(count == 1 and report[1] == "close", "Esc closes Files")
check(report[3], "Esc is reported while Yazi is hidden")
report, count = ends(0, "tab\n/browsed\n")
check(count == 1 and report[1] == "cd" and report[2] == "/browsed", "Tab moves Files")
check(not report[3], "Tab moves Files after Yazi shows again")
for _, code in ipairs({ 1, 11 }) do
  report, count = ends(code, "")
  check(count == 0, "status " .. code .. " leaves Files where it is")
end
-- A report that raises still shows Yazi again, and says why.
report_fails = true
local before = #notes
ends(0, "\n/picked\n")
report_fails = false
check(#notes == before + 1 and notes[#notes]:find("eon vanished", 1, true), "a failed report is reported")
print("quick-search guard: ok")
