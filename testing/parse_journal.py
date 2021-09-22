import pickle
import sys
import re

if len(sys.argv) < 2:
   print('usage: python ' + sys.argv[0] + ' JOURNAL_FILE')
   exit()

class Drone:
   def __init__(self, xbee_macaddr, upcore_macaddr, optitrack_id):
         self.xbee_macaddr = xbee_macaddr
         self.upcore_macaddr = upcore_macaddr
         self.optitrack_id = optitrack_id
         self.optitrack_data = None
         self.argos_log = None
         self.argos_logerr = None
         self.argos_socketaddr = None
         self.messages = None
         
   def log_as_utf8(self):
         try:
            return self.argos_log.decode()
         except UnicodeDecodeError:
            print('[error] could not decode log')
            return None

   def logerr_as_utf8(self):
         try:
            return self.argos_logerr.decode()
         except UnicodeDecodeError:
            print('[error] could not decode logerr')
            return None

# global dictionary of pipuck (indexed by robot id)
pipucks = {}
# global dictionary of drone (indexed by robot id)
drones = {}
# global dictionary of ARGoS logs (indexed by robot id)
argos_logs = {}
# global dictionary of messages (indexed by socket address)
messages = {}
# global dictionary of tracking data (indexed by rigid body id)
tracking_system = {}

# load journal file into local data structures
journal_file = open(sys.argv[1], 'rb')
while True:
   try:
      entry = pickle.load(journal_file)
      timestamp = entry['timestamp']
      event_type, event = entry['event']
      if event_type == 'TrackingSystem':
         for update in event:
            rigid_body_id = update['id']
            entry = {
               'timestamp': timestamp,
               'position': update['position'],
               'orientation': update['orientation']
            }
            if rigid_body_id in tracking_system:
               tracking_system[rigid_body_id].append(entry)
            else:
               tracking_system[rigid_body_id] = [entry]
      elif event_type == 'Descriptors':
         # note: this message should only be present once
         pipucks = {
            pipuck['id']: {
                  'rpi_macaddr': pipuck['rpi_macaddr'],
                  'optitrack_id': pipuck['optitrack_id'],
                  'apriltag_id': pipuck['apriltag_id']
            } for pipuck in event[0]
         }
         drones = {
            drone['id']: Drone(
                  drone['xbee_macaddr'],
                  drone['upcore_macaddr'],
                  drone['optitrack_id']
            ) for drone in event[1]
         }
      elif event_type == 'ARGoS':
         robot_id = event[0]
         if robot_id not in argos_logs:
            argos_logs[robot_id] = {
               'stdout': bytearray(),
               'stderr': bytearray(),
            }
         if event[1][0] == 'StandardOutput':
            argos_logs[robot_id]['stdout'].extend(event[1][1])
         elif event[1][0] == 'StandardError':
            argos_logs[robot_id]['stderr'].extend(event[1][1])
      elif event_type == 'Message':
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
socket_regex = re.compile("Connected to message router [0-9.:]+ from ([0-9.:]+)")

# build data structures for the drones
for drone_id, drone_obj in drones.items():
   # check if there is log data to be added
   if drone_id in argos_logs:
      drone_obj.argos_log = argos_logs[drone_id]['stdout']
      drone_obj.argos_logerr = argos_logs[drone_id]['stderr']
   else:
      print('[warning] no logs found for ' + drone_id)
   # extract the local socket address from the logs
   match = socket_regex.search(drone_obj.log_as_utf8())
   if match:
      drone_obj.socketaddr = match.group(1)
      if drone_obj.socketaddr in messages:
         drone_obj.messages = messages[drone_obj.socketaddr]
      else:
         print('[warning] no messages found for ' + drone_id)
   else:
      print('[warning] ARGoS did not report the socket address for ' + drone_id)
   # get the tracking system data
   if drone_obj.optitrack_id in tracking_system:
      drone_obj.optitrack_data = tracking_system[drone_obj.optitrack_id]
