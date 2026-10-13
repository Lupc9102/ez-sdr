'use strict';
// No receiver secrets or telemetry are persisted in browser storage.
const $ = id => document.getElementById(id);
let connected = false, token = '', control = null, status = null, refreshTimer = null;
let nextId = 100, audioContext = null, channelSignature = '';
const streams = new Map(), playing = new Map(), products = new Map();
const error = message => { $('error').textContent = String(message); $('error').hidden = !message; };
function report(promise) { promise.catch(e => error(e.message)); }
function element(tag, text, cls) { const e = document.createElement(tag); if (text !== undefined) e.textContent = text; if (cls) e.className = cls; return e; }
async function api(path, data, method) {
  if (!connected && (data !== undefined || method === 'DELETE')) throw new Error('Connect before changing the receiver.');
  const headers = {}; if (token) headers.Authorization = `Bearer ${token}`;
  if (data !== undefined) headers['Content-Type'] = 'application/json';
  const response = await fetch(`/api/${path}`, {method: method || (data === undefined ? 'GET' : 'POST'), headers, body: data === undefined ? undefined : JSON.stringify(data), cache:'no-store'});
  const text = await response.text(); let body = null;
  if (text) { try { body = JSON.parse(text); } catch { body = text; } }
  if (!response.ok) throw new Error(response.status === 401 ? 'Access denied. Enter the daemon access token.' : (body?.error || `${response.status}: ${text}`));
  return body;
}
function socket(path) {
  const url = `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}${path}`;
  const protocol = token ? 'ez-sdr-auth.' + [...new TextEncoder().encode(token)].map(v => v.toString(16).padStart(2,'0')).join('') : null;
  const ws = protocol ? new WebSocket(url, protocol) : new WebSocket(url);
  ws.binaryType = 'arraybuffer';
  ws.addEventListener('error', () => error('Stream connection failed. Check authentication and receiver status.'));
  return ws;
}
function subscribe(kind, id, receive) {
  if(!connected)return;
  const key = `${kind}/${id}`; if (streams.has(key)) return;
  const ws = socket(`/ws/stream/${key}`); streams.set(key, ws);
  ws.onmessage = event => { try { if(typeof event.data === 'string'){const message=JSON.parse(event.data);if(message.StreamGap){const gap=message.StreamGap;$('stream-status').hidden=false;$('stream-status').textContent=`${kind === 'spectrum' ? 'Spectrum' : 'Stream'} display skipped ${gap.dropped_frames} updates to keep up`;const playback=playing.get(id);if(playback&&audioContext)flushPlayback(id);return;}if(message.Error){error(message.Error.message);return;}if(message.ChannelRemoved){flushPlayback(id);playing.delete(id);return;}} receive(event.data); } catch(e) { error(e.message); } };
  ws.onclose = () => { if (streams.get(key) === ws) streams.delete(key); };
}
function unsubscribe(kind,id) { const key=`${kind}/${id}`; streams.get(key)?.close(); streams.delete(key); }
function disconnect() {
  connected=false; clearInterval(refreshTimer); refreshTimer=null;
  control?.close(); control=null;
  for (const id of playing.keys())flushPlayback(id);
  for (const ws of streams.values()) ws.close(); streams.clear(); playing.clear();
  if (audioContext) { audioContext.close(); audioContext=null; }
  token=''; $('stream-status').hidden=true; $('connection').textContent='Disconnected'; $('connection').className='badge';
  $('connect').disabled=false; $('disconnect').disabled=true;
}
async function connect() {
  disconnect(); token=$('token').value; error('');
  try {
    status=await api('status'); connected=true;
    $('frequency').value=status.hardware.frequency_hz; $('sample-rate').value=status.hardware.sample_rate_hz; $('gain').value=status.hardware.gain_db;
    $('connect').disabled=true; $('disconnect').disabled=false;
    $('connection').textContent='Connected'; $('connection').className='badge connected';
    control=socket('/ws/control');
    control.onopen=()=>error('');
    control.onmessage=event=>{
      const data=JSON.parse(event.data);
      if (data.Welcome) {
        const channel=data.Welcome.client_channels.spectrum;
        control.send(JSON.stringify({Subscribe:{channel:{id:channel,center_offset_hz:0,channel_sample_rate_hz:status.hardware.sample_rate_hz,kind:'Spectrum',demod_mode:null,audio:null,meteor_profile:null}}}));
        subscribe('spectrum',channel,drawSpectrum);
      } else if (data.Error) error(data.Error.message || data.Error);
    };
    control.onclose=()=>{ if (connected) { disconnect(); error('Control connection closed. Reconnect to resume streams.'); } };
    await refresh(); refreshTimer=setInterval(()=>report(refresh()),2000);
  } catch(e) { disconnect(); throw e; }
}
async function refresh() {
  if (!connected) return;
  const [updated,channels,recordings]=await Promise.all([api('status'),api('channels'),api('recordings')]); status=updated;
  for(const c of channels)c.recording=recordings.find(r=>r.channel_id===c.spec.id&&r.active);
  $('source').textContent=`${status.hardware.source_kind} · ${(status.hardware.sample_rate_hz/1e6).toFixed(3)} MS/s · ${(status.hardware.frequency_hz/1e6).toFixed(6)} MHz`;
  nextId=Math.max(nextId,...channels.filter(c=>c.spec.id<0x80000000).map(c=>c.spec.id+1));
  const signature=JSON.stringify(channels.map(c=>[c.spec,c.squelch_db,Boolean(c.recording?.active),playing.has(c.spec.id)]));
  const editing=$('channels').contains(document.activeElement)&&document.activeElement.matches('input,select,textarea');
  if(!editing&&signature!==channelSignature){renderChannels(channels);channelSignature=signature;}
}
function actionButton(label,run) { const b=element('button',label); b.type='button'; b.onclick=()=>report(Promise.resolve().then(run)); return b; }
function renderChannels(channels) {
  $('channels').replaceChildren(); $('radio-channels').replaceChildren();
  if (!channels.length) $('channels').append(element('p','No active channels.'));
  for (const c of channels) {
    const {id,kind,demod_mode}=c.spec;
    const row=element('div',undefined,'channel');
    row.append(element('span',`#${id} · ${kind}${demod_mode ? ' / '+demod_mode : ''} · ${Number(c.spec.center_offset_hz).toLocaleString()} Hz offset`));
    if (kind==='Audio') {
      row.append(actionButton(playing.has(id)?'Mute':'Listen',()=>toggleAudio(id)));
      const label=element('label','Squelch · dBFS'); const squelch=element('input'); squelch.type='number';squelch.min='-100';squelch.max='0';squelch.value=String(c.squelch_db ?? -100);squelch.onchange=()=>report(api(`channels/${id}/squelch`,{db:Number(squelch.value)})); label.append(squelch);row.append(label);
    } else if (kind==='AdsbPackets') row.append(actionButton('Show aircraft',()=>{showTab('aircraft'); subscribe('adsb-packets',id,drawAircraft);}));
    else if (kind==='LrptTelemetry') {
      row.append(actionButton('Show images',()=>{showTab('meteor'); subscribe('lrpt-telemetry',id,drawTelemetry);}));
      row.append(actionButton('Finalize pass',()=>{if(control?.readyState===WebSocket.OPEN)control.send(JSON.stringify({FinalizeTelemetryProduct:{channel_id:id}}));}));
      row.append(actionButton('New pass',()=>{if(control?.readyState===WebSocket.OPEN)control.send(JSON.stringify({StartTelemetryProduct:{channel_id:id}}));}));
    }
    row.append(actionButton(c.recording?.active ? 'Stop recording':'Record I/Q',async()=>{
      if (c.recording?.active) await api(`channels/${id}/recording/stop`,{});
      else await api(`channels/${id}/recording/start`,{format:'Cf32'});
      await refresh();
    }));
    row.append(actionButton('Remove',async()=>{await api(`channels/${id}`,undefined,'DELETE');unsubscribe('audio',id);unsubscribe('adsb-packets',id);unsubscribe('lrpt-telemetry',id);flushPlayback(id);playing.delete(id);await refresh();}));
    $('channels').append(row);
  }
}
async function addChannel(kind,offset,rate,mode,profile,audio) {
  const id=nextId++;
  await api('channels',{id,center_offset_hz:offset,channel_sample_rate_hz:rate,kind,demod_mode:mode,meteor_profile:profile,audio});
  await refresh(); return id;
}
function flushPlayback(id){const p=playing.get(id);if(!p)return;for(const source of p.sources){try{source.stop();}catch{}}p.sources.clear();if(audioContext)p.next=audioContext.currentTime;}
async function toggleAudio(id) {
  if(!connected)throw new Error('Connect before starting audio.');
  if (playing.has(id)) { flushPlayback(id);playing.delete(id);unsubscribe('audio',id);await refresh();return; }
  audioContext ||= new AudioContext(); await audioContext.resume();
  playing.set(id,{next:audioContext.currentTime, dropped:0,sources:new Set()});
  subscribe('audio',id,data=>{
    if (!(data instanceof ArrayBuffer)||data.byteLength<16) throw new Error('Invalid audio frame');
    const view=new DataView(data), rate=view.getUint32(4,true), count=view.getUint32(8,true), channels=view.getUint32(12,true);
    if (![1,2].includes(channels)||!rate||count%channels||16+count*4!==data.byteLength) throw new Error('Invalid audio stream geometry');
    const p=playing.get(id); if (!p||!audioContext||!count) return;
    if(p.next>audioContext.currentTime+.5){p.dropped++;return;}
    const buffer=audioContext.createBuffer(channels,count/channels,rate);
    for(let ch=0;ch<channels;ch++) { const dst=buffer.getChannelData(ch);for(let i=0;i<dst.length;i++)dst[i]=view.getFloat32(16+(i*channels+ch)*4,true); }
    const source=audioContext.createBufferSource();source.buffer=buffer;source.connect(audioContext.destination);p.sources.add(source);source.onended=()=>p.sources.delete(source);
    p.next=Math.max(p.next,audioContext.currentTime+.025);source.start(p.next);p.next+=buffer.duration;
  }); await refresh();
}
function drawSpectrum(data) {
  if (!(data instanceof ArrayBuffer)||data.byteLength<24) throw new Error('Invalid spectrum frame');
  const v=new DataView(data), center=Number(v.getBigUint64(0,true)),rate=v.getUint32(8,true),n=v.getUint32(20,true);
  if(!n||n>65536||24+n*4!==data.byteLength)throw new Error('Invalid spectrum dimensions');
  const canvas=$('spectrum'),ctx=canvas.getContext('2d'),w=canvas.width,h=canvas.height;
  ctx.clearRect(0,0,w,h);ctx.fillStyle='#7993ae';ctx.strokeStyle='#243247';ctx.font='12px system-ui';
  for(let db=-100;db<=0;db+=20){const y=-db*h/120;ctx.beginPath();ctx.moveTo(0,y);ctx.lineTo(w,y);ctx.stroke();ctx.fillText(`${db} dB`,8,y+14);}
  ctx.strokeStyle='#72dab4';ctx.beginPath(); const waterfall=$('waterfall'),wc=waterfall.getContext('2d');wc.drawImage(waterfall,0,0,w,waterfall.height-1,0,1,w,waterfall.height-1);
  for(let x=0;x<w;x++){let db=-120;const start=Math.floor(x*n/w),end=Math.max(start+1,Math.floor((x+1)*n/w));for(let i=start;i<Math.min(n,end);i++)db=Math.max(db,v.getFloat32(24+i*4,true));
    const y=(1-Math.max(0,Math.min(1,(db+120)/120)))*h;if(x===0)ctx.moveTo(x,y);else ctx.lineTo(x,y);
    const t=Math.max(0,Math.min(1,(db+100)/80));wc.fillStyle=`rgb(${Math.round(230*t*t)},${Math.round(190*t)},${Math.round(35+170*t)})`;wc.fillRect(x,0,1,1);
  }ctx.stroke();$('spectrum-label').textContent=`${(center/1e6).toFixed(6)} MHz · ${(rate/1e6).toFixed(3)} MHz span`;
  canvas.onclick=e=>{$('offset').value=Math.round(((e.clientX-canvas.getBoundingClientRect().left)/canvas.getBoundingClientRect().width-.5)*rate);};
}
function drawAircraft(data) {
  const snapshot=typeof data==='string'?JSON.parse(data):null;if(!snapshot||!Array.isArray(snapshot.aircraft))return;
  $('aircraft-rows').replaceChildren();const ctx=$('aircraft-map').getContext('2d'),w=$('aircraft-map').width,h=$('aircraft-map').height;ctx.clearRect(0,0,w,h);ctx.strokeStyle='#243247';
  for(let lon=-180;lon<=180;lon+=30){let x=(lon+180)*w/360;ctx.beginPath();ctx.moveTo(x,0);ctx.lineTo(x,h);ctx.stroke();}
  for(let lat=-90;lat<=90;lat+=30){let y=(90-lat)*h/180;ctx.beginPath();ctx.moveTo(0,y);ctx.lineTo(w,y);ctx.stroke();}
  ctx.font='12px system-ui';ctx.fillStyle='#72dab4';
  for(const a of snapshot.aircraft.slice(0,10000)){
    const row=element('tr');for(const value of [Number(a.icao).toString(16).toUpperCase().padStart(6,'0'),a.callsign||'—',a.altitude_ft ?? '—',a.ground_speed_kt == null?'—':Number(a.ground_speed_kt).toFixed(0),a.lat==null||a.lon==null?'—':`${Number(a.lat).toFixed(4)}, ${Number(a.lon).toFixed(4)}`])row.append(element('td',String(value)));$('aircraft-rows').append(row);
    if(Number.isFinite(a.lat)&&Number.isFinite(a.lon)){const x=(a.lon+180)*w/360,y=(90-a.lat)*h/180;ctx.beginPath();ctx.arc(x,y,3,0,Math.PI*2);ctx.fill();ctx.fillText(a.callsign||Number(a.icao).toString(16),x+7,y-5);}
  }
}
function drawTelemetry(data) {
  if (!(data instanceof ArrayBuffer)||data.byteLength<36)throw new Error('Invalid Meteor frame');
  const v=new DataView(data),id=v.getUint32(0,true),apid=v.getUint16(4,true),flags=v.getUint16(6,true),w=v.getUint32(8,true),h=v.getUint32(12,true),n=v.getUint32(24,true),product=v.getBigUint64(28,true).toString();
  const trailer=36+n;if(trailer>data.byteLength)throw new Error('Invalid Meteor pixel count');
  if(data.byteLength!==trailer){if(data.byteLength<trailer+4)throw new Error('Invalid Meteor metadata');const times=v.getUint32(trailer,true);if(times>16384||data.byteLength!==trailer+4+times*8||(times&&times!==h))throw new Error('Invalid Meteor line timestamps');}
  if(flags&8){$('meteor-status').textContent=[...products.values()].some(p=>p.channel===id)?'Meteor pass finalized; channel images are ready to export':'Meteor pass finalized; no channel images decoded';return;}
  if(!apid&&!w&&!h){$('meteor-status').textContent='Receiving a new Meteor pass';for(const [key,p] of products)if(p.channel===id){p.section.remove();products.delete(key);}return;}
  if(!w||w>2048||!h||h>16384||w*h!==n)throw new Error('Invalid Meteor image dimensions');
  const key=`${id}/${apid}`;let p=products.get(key);
  if(p&&p.product!==product){p.section.remove();products.delete(key);p=null;}
  if(!p){const section=element('div',undefined,'image-product'),heading=element('h2'),canvas=element('canvas'),save=actionButton('Export channel PNG',()=>saveImage(key));section.append(heading,save,canvas);$('meteor-images').append(section);p={channel:id,product,section,heading,canvas};products.set(key,p);}
  $('meteor-status').textContent=(flags&4)?'Full Meteor channel received':'Receiving Meteor images';
  p.width=w;p.height=h;p.pixels=new Uint8Array(data,36).slice();p.heading.textContent=`Channel ${apid} · ${w} × ${h} · RS ${v.getUint32(16,true)} accepted / ${v.getUint32(20,true)} failed`;
  const scale=Math.min(1,768/w,4096/h),pw=Math.max(1,Math.floor(w*scale)),ph=Math.max(1,Math.floor(h*scale));p.canvas.width=pw;p.canvas.height=ph;
  const ctx=p.canvas.getContext('2d'),image=ctx.createImageData(pw,ph);for(let y=0;y<ph;y++)for(let x=0;x<pw;x++){const gray=p.pixels[Math.floor(y/scale)*w+Math.floor(x/scale)],i=(y*pw+x)*4;image.data[i]=image.data[i+1]=image.data[i+2]=gray;image.data[i+3]=255;}ctx.putImageData(image,0,0);
}
function saveImage(key){const p=products.get(key);if(!p)return;const canvas=document.createElement('canvas');canvas.width=p.width;canvas.height=p.height;const ctx=canvas.getContext('2d'),image=ctx.createImageData(p.width,p.height);for(let i=0;i<p.pixels.length;i++){const j=i*4;image.data[j]=image.data[j+1]=image.data[j+2]=p.pixels[i];image.data[j+3]=255;}ctx.putImageData(image,0,0);canvas.toBlob(blob=>{if(!blob)return;const url=URL.createObjectURL(blob),a=element('a');a.href=url;a.download=`meteor-${p.product}-${key.replace('/','-')}.png`;a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);});}
function showTab(name){for(const id of ['radio','aircraft','meteor'])$(id).hidden=id!==name;for(const b of document.querySelectorAll('.tab'))b.classList.toggle('active',b.dataset.tab===name);}
$('connect').onclick=()=>report(connect());$('disconnect').onclick=disconnect;$('refresh').onclick=()=>report(refresh());
for(const b of document.querySelectorAll('.tab'))b.onclick=()=>showTab(b.dataset.tab);
$('capture').onsubmit=e=>{e.preventDefault();report((async()=>{await api('hardware/sample-rate',{hz:Number($('sample-rate').value)});await api('hardware/frequency',{hz:Number($('frequency').value)});await api('hardware/gain',{db:Number($('gain').value)});await refresh();})());};
const widths={Wfm:180000,Fm:25000,Am:15000,Usb:3300,Lsb:3300,Dsb:4600,Cw:200,Raw:0};$('mode').onchange=()=>$('rf-width').value=widths[$('mode').value];
$('add-radio').onsubmit=e=>{e.preventDefault();report((async()=>{const mode=$('mode').value,rf=Number($('rf-width').value),rate=mode==='Wfm'?256000:200000;const id=await addChannel('Audio',Number($('offset').value),rate,mode,null,{rf_bandwidth_hz:rf,output_sample_rate_hz:48000,stereo:mode==='Wfm'&&$('stereo').checked,cw_tone_hz:Number($('cw-tone').value)});await toggleAudio(id);})());};
$('add-aircraft').onsubmit=e=>{e.preventDefault();report((async()=>{const id=await addChannel('AdsbPackets',Number($('adsb-offset').value),2400000,null,null,null);subscribe('adsb-packets',id,drawAircraft);})());};
$('add-meteor').onsubmit=e=>{e.preventDefault();report((async()=>{const profile=$('meteor-profile').value,id=await addChannel('LrptTelemetry',Number($('meteor-offset').value),profile==='Oqpsk72'?288000:320000,null,profile,null);subscribe('lrpt-telemetry',id,drawTelemetry);})());};
window.addEventListener('beforeunload',disconnect);
