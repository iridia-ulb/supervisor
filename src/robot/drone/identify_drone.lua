function init()
   robot.leds.set_leds("red")
   count = 0
end

function step()
   count = count + 1
   if count == 3 then
      robot.leds.set_leds("black")
   end
end

function reset()
end

function destroy()
end
