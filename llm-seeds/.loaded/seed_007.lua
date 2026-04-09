
-- 7: pcall + error inside __gc + collectgarbage in tostring
local boom = newproxy(true)
local mt = getmetatable(boom)
mt.__gc = function(self)
  local ok, err = pcall(function()
    local msg = setmetatable({}, {
      __tostring = function()
        collectgarbage("collect")
        return string.rep("E", 200)
      end
    })
    error(msg)
  end)
  collectgarbage("step", 2)
  local _ = tostring(err)
end
boom = nil
collectgarbage("collect")
collectgarbage("collect")
local ok, err = pcall(function()
  collectgarbage("collect")
  error("outer")
end)
local _ = tostring(err)
