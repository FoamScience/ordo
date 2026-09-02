-- no-globals: leaks into the global namespace
bad_global = "leaked"

-- compliant: scoped to the file
local ok_local = "scoped"

-- flat-control-flow: three levels of nested conditionals
local function validate(a, b, c)
  if a then
    if b then
      if c then
        return true
      end
    end
  end
  return false
end

-- compliant: same check, returned early instead of nested
local function validate_early(a, b, c)
  if not a then
    return false
  end
  if not b then
    return false
  end
  if not c then
    return false
  end
  return true
end

-- three-arguments: more than 3 parameters
local function configure(host, port, timeout, retries)
  return host, port, timeout, retries
end

-- compliant: within the limit
local function connect(host, port)
  return host, port
end

-- no-function-in-table: function literal defined inline as a table value
local handlers = {
  greet = function(name)
    return "hello " .. name
  end,
}

-- compliant: defined outside, referenced by name
local function farewell(name)
  return "bye " .. name
end

local handlers_ok = {
  farewell = farewell,
}

-- function-size: over 40 lines
local function long_report(a, b)
  local total = a + b
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  total = total + 1
  return total
end
