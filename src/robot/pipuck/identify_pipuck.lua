function init()
   robot.leds.set_ring_leds(true)
   count = 0
end

function step()
   count = count + 1
   if count == 3 then
      robot.leds.set_ring_leds(false)
   end
end

function reset()
end

function destroy()
end
