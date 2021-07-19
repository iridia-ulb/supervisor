import pickle
import sys
import re

if len(sys.argv) < 2:
   print('usage: python ' + sys.argv[0] + ' JOURNAL_FILE')
   exit()

class Robot:
   def __init__(self):
         self.log = bytearray()
         self.addr = None
   
   def log_as_utf8(self):
         try:
            return self.log.decode()
         except UnicodeDecodeError:
            print('[error] could not decode log file')
            return None

# global dictionary of robots
robots = {}
# global dictionary of messages
messages = {}

# go through all entries in the journal
journal_file = open(sys.argv[1], 'rb')
while True:
   try:
      entry = pickle.load(journal_file)
      timestamp = entry['timestamp']
      timestamp = timestamp['secs'] * 10e9 + timestamp['nanos']
      event_type, event = entry['event']
      if event_type == 'Robot':
         robot_id = event[0]
         if robot_id not in robots:
            robots[robot_id] = Robot()
         robot_event_type, robot_event_data = event[1]
         robots[robot_id].log.extend(robot_event_data)
      elif event_type == 'Broadcast':
         source = event[0]
         message = {
            'timestamp': timestamp,
            'data': event[1],
         }
         if source in messages:
            messages[source].append(message)
         else:
            messages[source] = [message]
   except EOFError:
      break

# Regex for extracting the local socket address used by each robot
robot_socket_regex = re.compile("Connected to message router [0-9.:]+ from ([0-9.:]+)")
# Use regex to extract the local socket address of each robot and associate it with the
# messages that were broadcasted
for robot_id, robot in robots.items():
   match = robot_socket_regex.search(robot.log_as_utf8())
   if match:
      robot.addr = match.group(1)
      if robot.addr in messages:
         robot.messages = messages[robot.addr]
      else:
         print('[warning] no messages found for robot ' + robot_id)
   else:
      print('[error] could not extract robot socket address from log')

