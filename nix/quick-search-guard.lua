-- Checks the Files pair's quick-search plugin against stand-ins for Yazi, Zoxide
-- and a slow fzf (EON-C18): a second invocation while one is up returns at once
-- instead of queueing a second fzf, every exit frees the plugin for the next
-- one, the runtime's fzf arguments reach fzf as given and only with a marker,
-- the marker is removed before fzf starts and after it ends, fzf keeps its last
-- frame, and a pick or Esc is reported while Yazi is still hidden, so Yazi does
-- not redraw Files before the pane. Whether Venus shows a frame is for the
-- installed proof; real fzf, Yazi and Orbit are in the runtime's real-PTY tests.
local plugin, marker = arg[1], arg[2]
-- Stand-ins for the runtime's arguments and leave status, which its own tests check.
local leave_status = "42"
local fzf_args = { "--with-shell=/bin/sh -c", "--bind=start:execute-silent(mark)", "--bind=key:become(exit " .. leave_status .. ")" }
local marker_set = true
local startup_notice

local function exists(path)
  local file = io.open(path)
  if file then file:close() end
  return file ~= nil
end

local state, spawned, notes, reports = {}, 0, {}, {}
local hidden = false
local fzf_options -- FZF_DEFAULT_OPTS of the last fzf
local fzf_arguments -- the arguments of the last fzf
local marker_at_spawn -- whether a marker existed as the last fzf started
local report_fails = false
local fzf_wait -- how the next fzf ends
local zoxide_fails = false
local during_close
local before_sync
local before_report

ya = {
  sync = function(f) return function(...)
    if before_sync then
      before_sync = nil
      coroutine.yield("before sync")
    end
    return f(state, ...)
  end end,
  notify = function(note) notes[#notes + 1] = note.content end,
  emit = function(name, args)
    reports[#reports + 1] = { name, args[1], hidden, args }
  end,
}
ui = {
  hide = function()
    hidden = true
    return { drop = function()
      if during_close then during_close() end
      hidden = false
    end }
  end,
}
package.loaded["tab-folder"] = {
  set = function(dir)
    if before_report then before_report() end
    if report_fails then error("eon vanished") end
    reports[#reports + 1] = { "set", dir, hidden }
  end,
  close = function()
    if before_report then before_report() end
    reports[#reports + 1] = { "close", nil, hidden }
  end,
}
cx = { active = { current = { cwd = "/" } } }

local builder = {}
builder.__index = builder
for _, method in ipairs({ "stdin", "stdout", "stderr" }) do
  builder[method] = function(self) return self end
end
function builder:arg(arguments)
  if self.name == "fzf" then fzf_arguments = arguments end
  return self
end
function builder:env(name, value)
  if self.name == "fzf" and name == "FZF_DEFAULT_OPTS" then fzf_options = value end
  if self.name == "fzf" and name == "EON_QUICK_TOKEN" then self.token = value end
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
  marker_at_spawn = exists(marker)
  local wait = fzf_wait
  return {
    -- A slow fzf: its start action creates the marker, and it ends only when
    -- resumed.
    wait_with_output = function()
      if marker_set then
        local ready = assert(io.open(marker, "w"))
        ready:write(self.token or "")
        ready:close()
      end
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
  if name == "EON_QUICK_MARKER" then return marker_set and marker or "" end
  if name == "EON_QUICK_FZF_ARGS" then return table.concat(fzf_args, "\n") end
  if name == "EON_QUICK_LEAVE_STATUS" then return leave_status end
  if name == "EON_FILES_NOTICE" then return startup_notice end
  return getenv(name)
end

local M = dofile(plugin)
local function invocation(token)
  return coroutine.create(function() M:entry({ args = { token } }) end)
end
local function check(condition, message)
  if not condition then
    io.stderr:write("FAIL: " .. message .. "\n")
    os.exit(1)
  end
end

-- The runtime's switch to Files ends fzf with EON_QUICK_LEAVE_STATUS. A marker
-- an earlier fzf left behind is gone before this one starts.
io.open(marker, "w"):close()
fzf_wait = function() return { status = { code = tonumber(leave_status), success = false }, stdout = "", stderr = "" } end
local first = invocation()
check(select(2, coroutine.resume(first)) == "fzf up", "the first quick search starts fzf")
check(marker_at_spawn == false, "a stale marker is removed before fzf starts")
check(exists(marker), "fzf's start created the marker")
local given = {}
for _, argument in ipairs(fzf_arguments) do
  if argument:sub(1, 9) ~= "--header=" then given[#given + 1] = argument end
end
check(table.concat(given, "\n") == table.concat(fzf_args, "\n"),
  "the runtime's fzf arguments reach fzf as given: " .. table.concat(given, " | "))
check(fzf_options and fzf_options:find("--no-clear", 1, true), "fzf keeps its last frame when it ends")
-- Alt+Z again before the first is done.
local second = invocation()
check(coroutine.resume(second), "the second invocation runs")
check(coroutine.status(second) == "dead", "the second invocation returns at once")
check(spawned == 1, "the second invocation starts no second fzf, started " .. spawned)
check(coroutine.resume(first) and coroutine.status(first) == "dead", "the first ends")
check(not exists(marker), "the marker is removed when fzf ends")

local function request(token)
  local path = marker .. "." .. token .. ".pending"
  io.open(path, "w"):close()
  return path
end

local cancelled = request("12-345-0")
local claiming = invocation("12-345-0")
before_sync = true
check(select(2, coroutine.resume(claiming)) == "before sync", "the claim awaits Yazi")
check(os.remove(cancelled), "hide retires the token before Yazi claims")
check(coroutine.resume(claiming) and coroutine.status(claiming) == "dead" and not exists(marker),
  "a cancelled claim cannot start fzf")

-- Each exit frees the plugin: a failed Zoxide query, which also removes a
-- marker an earlier fzf left...
io.open(marker, "w"):close()
zoxide_fails = true
local failed = request("12-345-4")
check(coroutine.resume(invocation("12-345-4")), "a failing invocation runs")
zoxide_fails = false
check(not exists(failed), "a failed search settles its startup request")
check(#notes == 1, "the failed query is reported")
check(not exists(marker), "a failed query leaves no stale marker")
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
local function ends(code, stdout, stderr)
  fzf_wait = function()
    return { status = { code = code, success = code == 0 }, stdout = stdout, stderr = stderr or "" }
  end
  reports = {}
  local token = marker_set and "12-345-987" or nil
  local pending = token and request(token)
  before_report = function()
    check((not pending or not exists(pending)) and not exists(marker),
      "a callback into Runtime cannot hold its own startup or ready marker")
  end
  local run = invocation(token)
  coroutine.resume(run)
  coroutine.resume(run)
  before_report = nil
  check(coroutine.status(run) == "dead", "quick search ends after status " .. tostring(code))
  check(not hidden, "Yazi shows again after status " .. tostring(code))
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
-- A report that raises still shows Yazi again, and says why.
report_fails = true
local before = #notes
ends(0, "\n/picked\n")
report_fails = false
check(#notes == before + 1 and notes[#notes]:find("eon vanished", 1, true), "a failed report is reported")
-- A marker fzf leaves after an error is removed too.
fzf_wait = function() error("fzf vanished") end
local raising = invocation()
coroutine.resume(raising)
coroutine.resume(raising)
check(not exists(marker), "the marker is removed when fzf raises")

-- Without a marker, fzf gets none of the runtime's arguments.
marker_set = false
fzf_wait = function() return { status = { code = 1, success = false }, stdout = "", stderr = "" } end
local bare = invocation()
coroutine.resume(bare)
coroutine.resume(bare)
for _, argument in ipairs(fzf_arguments) do
  check(argument:sub(1, 9) == "--header=", "no runtime argument without a marker: " .. argument)
end
marker_set = true
fzf_wait = function() return { status = { code = tonumber(leave_status), success = false }, stdout = "", stderr = "" } end
local active = invocation()
check(select(2, coroutine.resume(active)) == "fzf up", "the older search opens")
local duplicate = request("12-345-1")
check(coroutine.resume(invocation("12-345-1")), "a Runtime duplicate reaches the plugin")
check(not exists(duplicate) and exists(marker), "rejection settles only the duplicate")
local reopen = request("12-345-2")
local count = #reports
during_close = function()
  check(not exists(marker), "fzf ended before reopen arrives")
  check(coroutine.resume(invocation("12-345-2")), "reopen reaches the closing search")
  check(coroutine.resume(invocation()), "plain duplicate reaches the closing search")
  check(exists(reopen) and #reports == count, "reopen stays pending through teardown")
end
check(coroutine.resume(active), "the older search closes")
during_close = nil
check(#reports == count + 1 and reports[count + 1][1] == "plugin", "exactly one reopen is emitted")
local reopen_args = reports[count + 1][4]
check(reopen_args[1] == "quick-search" and reopen_args[2] == "12-345-2", "the queued token is preserved")
local reopened = invocation(reopen_args[2])
check(select(2, coroutine.resume(reopened)) == "fzf up", "the queued search opens")
local ready = assert(io.open(marker))
check(ready:read("a") == reopen_args[2], "fzf publishes the admitted token")
ready:close()
check(coroutine.resume(reopened) and not exists(reopen), "reopen settles its own token")

local old = invocation("initial")
check(select(2, coroutine.resume(old)) == "fzf up", "initial search opens with no token after recovery")
local expired = request("12-345-3")
count = #reports
local before = spawned
during_close = function()
  check(coroutine.resume(invocation("12-345-3")), "a Runtime reopen waits for release")
  check(os.remove(expired), "clear the pending request before re-emitting")
end
check(coroutine.resume(old), "initial search closes")
during_close = nil
check(#reports == count and spawned == before, "an expired reopen never opens")
check(coroutine.resume(invocation("12-345-3")) and spawned == before, "late deliveries are inert")

local transform = [[--bind=start:+transform-header([ -n "$EON_FILES_CLI" ] && [ -f "$EON_QUICK_MARKER" ] && "$EON_FILES_CLI" --eon-initial-picker-header initial || printf '%s' "$EON_FILES_NOTICE")]]
local function check_header(token, header, action)
  local run = invocation(token)
  check(select(2, coroutine.resume(run)) == "fzf up", "the header case starts fzf")
  local expected = { table.unpack(fzf_args) }
  if header then expected[#expected + 1] = "--header=" .. header end
  if action then expected[#expected + 1] = transform end
  check(table.concat(fzf_arguments, "\n") == table.concat(expected, "\n"),
    "startup header preserves the ready/leave bindings: " .. table.concat(fzf_arguments, " | "))
  check(coroutine.resume(run) and coroutine.status(run) == "dead", "the header case finishes")
end

state, startup_notice = {}, nil
check_header("initial")
state, startup_notice = {}, ""
check_header("initial")
state = {}
request("initial")
check_header("initial", nil, true)
state, startup_notice = {}, "Peer window"
request("initial")
check_header("initial", startup_notice, true)
check_header("initial")
state = {}
request("12-345-9")
check_header("12-345-9", startup_notice)

-- Status 1, EON_QUICK_LEAVE_STATUS bounds (3..=125), mismatch, unset, bare mode, and signal exit.
for _, c in ipairs({
  { "42", 1, true, true }, { "3", 3, true, true }, { "125", 125, true, true }, { "42", 42, true, true },
  { "42", 43, true, false }, { "2", 2, true, false }, { "126", 126, true, false },
  { "127", 127, true, false }, { "128", 128, true, false }, { "255", 255, true, false },
  { nil, 42, true, false }, { "42", 42, false, false }, { nil, nil, true, false }, { "42", nil, true, false },
}) do
  leave_status, marker_set = c[1], c[3]
  local n0, s0 = #notes, spawned
  report, count = ends(c[2], "", "fzf-sentinel-err")
  local tag = tostring(c[1]) .. "/" .. tostring(c[2]) .. "/marker=" .. tostring(c[3])
  check(spawned == s0 + 1 and count == 0 and #notes == n0 + (c[4] and 0 or 1)
    and (c[4] or notes[#notes] == "fzf-sentinel-err"), "leave case " .. tag)
end
print("quick-search guard: ok")
