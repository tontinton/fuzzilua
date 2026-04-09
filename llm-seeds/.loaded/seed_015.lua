
-- 15: setfenv on function during __gc, then call it
local target_fn = function() return "original" end
local env_backup = getfenv(target_fn)
local captured_fn = target_fn
local call_results = {}
local t = newproxy(true)
getmetatable(t).__gc = function(self)
  local new_env = setmetatable({}, {
    __index = function(t, k)
      collectgarbage("step", 0)
      return env_backup[k]
    end
  })
  setfenv(captured_fn, new_env)
  local ok, ret = pcall(captured_fn)
  call_results[1] = ok
  call_results[2] = ret
  collectgarbage("step", 2)
  setfenv(captured_fn, new_env)
  local ok2, ret2 = pcall(captured_fn)
  call_results[3] = ok2
end
t = nil
collectgarbage("collect")
collectgarbage("collect")
local ok, val = pcall(target_fn)
call_results[4] = val
local _ = #call_results

