#!/usr/bin/env python3
"""Generate one second of synthetic CU8 PPM, suitable for native ADS-B replay.

Known even/odd airborne CPR frames encode aircraft 40621D. No aircraft objects
are inserted into the GUI. Run this file, then replay /tmp/ez-final-adsb-demo.cu8
at 2.4 MSPS and 1090 MHz, with Loop enabled. This is synthetic, not off-air.
"""
import hashlib,json,os
from pathlib import Path
os.sched_setaffinity(0,set(range(6)))
os.setpriority(os.PRIO_PROCESS,0,max(10,os.getpriority(os.PRIO_PROCESS,0)))
messages=['8D40621D58C382D690C8AC2863A7','8D40621D58C386435CC412692AD6']
frames=[]
for hexmsg in messages:
    bits=''.join(f'{byte:08b}' for byte in bytes.fromhex(hexmsg))
    pulses=[(t,t+.5) for t in [0,1,3.5,4.5]]+[(8+k+(0 if b=='1' else .5),8+k+(0 if b=='1' else .5)+.5) for k,b in enumerate(bits)]
    data=bytearray()
    for n in range(48000):
        lo=(n-200-.375)/2.4;hi=lo+1/2.4
        fraction=sum(max(0,min(hi,end)-max(lo,start)) for start,end in pulses)/(hi-lo) if 0<=lo<121 else 0
        data.extend([round(127.4+127.4*.65*fraction),127])
    frames.append(data)
iq=b''.join(bytes(frames[n%2]) for n in range(50))
path=Path('/tmp/ez-final-adsb-demo.cu8');path.write_bytes(iq)
manifest={'provenance':'Independent synthetic PPM from known even/odd CPR frames; no injected GUI aircraft','path':str(path),'sample_rate_hz':2400000,'center_frequency_hz':1090000000,'bytes':len(iq),'sha256':hashlib.sha256(iq).hexdigest(),'frames':messages,'expected_icao':'40621D','expected_latitude':52.257202,'expected_longitude':3.919373}
(Path(__file__).resolve().parent/'adsb-native-fixture.json').write_text(json.dumps(manifest,indent=2)+'\n')
print(json.dumps(manifest,indent=2))
