#!/usr/bin/env python3
"""Independent integrated-PPM/noise generator; compare actual native decoders."""
import hashlib,json,math,os,random,re,socket,subprocess,time
from pathlib import Path
A=Path(__file__).resolve().parent; R=A.parents[1]
os.sched_setaffinity(0,set(range(6)));os.setpriority(os.PRIO_PROCESS,0,max(10,os.getpriority(os.PRIO_PROCESS,0)))
def crc(data):
    rem=0
    for byte in data:
        rem^=byte<<16
        for _ in range(8):rem=((rem<<1)^0xfff409 if rem&0x800000 else rem<<1)&0xffffff
    return rem
rng=random.Random(1011);iq=bytearray();cases=[]
for amplitude in [.8,.4,.2,.1,.05]:
 for noise in [0,.01,.035]:
  for phase in range(8):
   n=len(cases); base=bytes.fromhex('8D40621D58C382D690C8AC')
   base=base[:1]+(0xa00000+n).to_bytes(3,'big')+base[4:]
   msg=base+crc(base).to_bytes(3,'big');bits=''.join(f'{b:08b}' for b in msg)
   pulses=[(v,v+.5) for v in [0,1,3.5,4.5]]+[(8+k+(0 if b=='1' else .5),8+k+(0 if b=='1' else .5)+.5) for k,b in enumerate(bits)]
   for sample in range(700):
    lo=(sample-200-phase/8)/2.4;hi=lo+1/2.4
    energy=sum(max(0,min(hi,end)-max(lo,start)) for start,end in pulses)/(hi-lo)
    level=amplitude*energy
    for component in [level+noise*rng.gauss(0,1),noise*rng.gauss(0,1)]:iq.append(max(0,min(255,round(127.4+127.4*component))))
   cases.append(dict(hex=msg.hex().upper(),amplitude=amplitude,noise_rms=noise,phase_samples=phase/8))
# More than one reader block and a quiet tail avoid end-of-buffer differences.
iq+=bytes([127,127])*140000
fixture=Path('/tmp/ez-final-adsb.uc8');fixture.write_bytes(iq)
reference=Path('/tmp/ez-sdr-final-dump1090-reference/dump1090')
cp=subprocess.run([str(reference),'--ifile',str(fixture),'--iformat','UC8','--raw','--no-fix'],capture_output=True,timeout=60)
(A/'evidence/adsb-reference.stdout.txt').write_bytes(cp.stdout);(A/'evidence/adsb-reference.stderr.txt').write_bytes(cp.stderr)
reference_frames=re.findall(rb'\*([0-9A-Fa-f]{28});',cp.stdout)
def port():
 s=socket.socket();s.bind(('127.0.0.1',0));p=s.getsockname()[1];s.close();return p
ports=[port() for _ in range(3)]
command=[str(R/'target/release/dump1090'),'--ifile','-','--net','--net-bind-address','127.0.0.1','--net-beast-port',str(ports[0]),'--net-sbs-port',str(ports[1]),'--net-raw-port',str(ports[2])]
log=(A/'evidence/adsb-rust.stderr.txt').open('wb');p=subprocess.Popen(command,stdin=subprocess.PIPE,stdout=subprocess.DEVNULL,stderr=log)
connections=[]
try:
 for number in ports:
  for _ in range(100):
   try:s=socket.create_connection(('127.0.0.1',number),timeout=.1);break
   except OSError:time.sleep(.05)
  else:raise RuntimeError('listener did not start')
  s.settimeout(20);connections.append(s)
 time.sleep(.2);p.stdin.write(iq);p.stdin.close()
 streams=[]
 for s in connections:
  data=bytearray()
  while True:
   part=s.recv(65536)
   if not part:break
   data+=part
  streams.append(bytes(data))
 rc=p.wait(timeout=20)
finally:
 if p.poll() is None:p.kill();p.wait()
 for s in connections:s.close()
 log.close()
raw=streams[0];frames=[];i=0
while i<len(raw)-1:
 if raw[i]!=26 or raw[i+1] not in [49,50,51]:i+=1;continue
 kind=raw[i+1];length={49:9,50:14,51:21}[kind];i+=2;payload=bytearray()
 while len(payload)<length and i<len(raw):
  value=raw[i];i+=1
  if value==26 and i<len(raw) and raw[i]==26:i+=1
  payload.append(value)
 if kind==51 and len(payload)==21:frames.append(bytes(payload[7:]).hex().upper())
refset={v.decode().upper() for v in reference_frames};ours=set(frames);expected={c['hex'] for c in cases}
for c in cases:c.update(reference=c['hex'] in refset,rust=c['hex'] in ours)
summary={'source':'Independent Python pulse integration, fixed seed 1011; synthetic RF, not off-air sensitivity proof','settings':'Both primary runs disable CRC correction; Rust CLI default nfix_crc=0, C --no-fix. Reference EOF exits 1 by readerThreadEntryPoint design; frame output is retained.','cases':len(cases),'sample_rate_hz':2400000,'fixture_sha256':hashlib.sha256(iq).hexdigest(),'rust_binary_sha256':hashlib.sha256(Path(command[0]).read_bytes()).hexdigest(),'reference_binary_sha256':hashlib.sha256(reference.read_bytes()).hexdigest(),'reference_command':[str(reference),'--ifile',str(fixture),'--iformat','UC8','--raw','--no-fix'],'rust_command':command,'reference_commit':subprocess.check_output(['git','-C',str(reference.parent),'rev-parse','HEAD'],text=True).strip(),'reference_exit':cp.returncode,'rust_exit':rc,'reference_recovered':len(refset&expected),'rust_recovered':len(ours&expected),'both':len(refset&ours&expected),'reference_only':len((refset-ours)&expected),'rust_only':len((ours-refset)&expected),'reference_unexpected':sorted(refset-expected),'rust_unexpected':sorted(ours-expected),'cli_stream_bytes':dict(zip(['beast','sbs','raw'],map(len,streams))),'cases_detail':cases}
(A/'adsb-comparison.json').write_text(json.dumps(summary,indent=2)+'\n')
print(json.dumps({k:v for k,v in summary.items() if k!='cases_detail'},indent=2))
