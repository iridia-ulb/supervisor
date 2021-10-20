pid = io.open('/proc/self/stat'):read("*number")


function stats(samples)
   local cnt = 0
   local avg = 0
   for k,v in pairs(samples) do
      avg = avg + v
      cnt = cnt + 1
   end
   if cnt > 0 then
      avg = avg / cnt
      local min = math.min(table.unpack(samples))
      local max = math.max(table.unpack(samples))      
      return min, avg, max   
   else
      return nil, nil, nil
   end
end

function init()
   count = 0
   results = {}
end

function step()
   count = count + 1
   -- send ping
   local data = {
      type = "ping",
      count = count,
      pid = pid
   }
   robot.wifi.tx_data(data)
   
   for index, message in ipairs(robot.wifi.rx_data) do
      -- got ping
      if message.type == "ping" then
         -- send back pong with same pid and count
         local data = {
            type = "pong",
            count = message.count,
            pid = message.pid
         }
         robot.wifi.tx_data(data)
      -- got pong
      elseif message.type == "pong" and message.pid == pid then
         -- message.count is the step that the message was sent on
         delay = count - message.count
         if results[message.count] == nil then
            results[message.count] = { delay }
         else
            table.insert(results[message.count], delay)
         end
      end
   end
end

function reset()
end

function destroy()
   output = io.open(pid .. ".csv", "w")
   output:write(string.format("step,min,avg,max\n"))
   for step, delays in ipairs(results) do
      min, avg, max = stats(delays)
      output:write(string.format("%d,%.1f,%.1f,%.1f\n", step, min, avg, max))
   end
   output:flush()
   output:close()
end
