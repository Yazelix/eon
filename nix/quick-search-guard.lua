-- Checks the Files pair's quick-search plugin against stand-ins for Yazi, Zoxide
-- and a slow fzf (EON-C18): a second invocation while one is up returns at once
-- instead of queueing a second fzf, every exit frees the plugin for the next
-- one, and fzf's socket is removed when it ends.
-- Usage: lua quick-search-guard.lua PLUGIN SOCKET
local plugin, socket = arg[1], arg[2]

local state, spawned, notes = {}, 0, {}
local fzf_wait -- how the next fzf ends
local zoxide_fails = false

ya = {
  sync = function(f) return function(...) return f(state, ...) end end,
  notify = function(note) notes[#notes + 1] = note.content end,
  emit = function() end,
}
ui = { hide = function() return { drop = function() end } end }
cx = { active = { current = { cwd = "/" } } }

local builder = {}
builder.__index = builder
for _, method in ipairs({ "arg", "env", "stdin", "stdout", "stderr" }) do
  builder[method] = function(self) return self end
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
print("quick-search guard: ok")
