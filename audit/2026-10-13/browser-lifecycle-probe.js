'use strict';
const fs = require('fs');
const vm = require('vm');
const assert = require('assert');
class Element {
  constructor() { this.value = ''; this.textContent = ''; this.hidden = false; this.disabled = false; this.className = ''; this.classList = {toggle(){}}; }
  contains(){return false;}
  replaceChildren(){}
  append(){}
}
const elements = new Map();
const document = {getElementById(id){if(!elements.has(id))elements.set(id,new Element());return elements.get(id);},querySelectorAll(){return [];},createElement(){return new Element();},activeElement:new Element()};
const sockets = [];
class Socket {
  static OPEN = 1;
  constructor(url){this.url=url;this.readyState=1;this.closed=false;sockets.push(this);}
  addEventListener(){}
  close(){this.closed=true;/* Closing event is deliberately delivered later, as by a browser. */}
  send(){}
}
const context = {document,location:{protocol:'http:',host:'127.0.0.1:7891'},WebSocket:Socket,TextEncoder,Map,Uint8Array,DataView,ArrayBuffer,window:{addEventListener(){}},setInterval(){return 1;},clearInterval(){},setTimeout(){},fetch:async url=>({ok:true,text:async()=>JSON.stringify(url.endsWith('/status')?{hardware:{frequency_hz:100000000,sample_rate_hz:2048000,gain_db:0,source_kind:'Synthetic'}}:[])})};
vm.createContext(context);
vm.runInContext(fs.readFileSync('ez-daemon/src/web/frontend/app.js','utf8'),context);
(async()=>{
  await vm.runInContext('connect()',context);
  const obsolete = sockets[0];
  vm.runInContext('disconnect()',context);
  await vm.runInContext('connect()',context);
  const replacement = sockets.at(-1);
  assert.equal(vm.runInContext('connected',context),true);
  obsolete.onclose();
  assert.equal(vm.runInContext('connected',context),false);
  assert.equal(replacement.closed,true);
  assert.equal(document.getElementById('error').textContent,'Control connection closed. Reconnect to resume streams.');
  vm.runInContext("products.set('100/64',{channel:100,product:'123',pixels:new Uint8Array(1024)});disconnect()",context);
  assert.equal(vm.runInContext('products.size',context),1);
  console.log(JSON.stringify({source:'ez-daemon/src/web/frontend/app.js',stale_close_disconnects_new_session:true,disconnect_retains_meteor_products:true},null,2));
})().catch(e=>{console.error(e);process.exitCode=1;});
