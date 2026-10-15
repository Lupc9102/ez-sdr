#!/usr/bin/env python3
"""Behavioral audit probes; assertions preserve the audited defects, not desired behavior.
Run cargo test --workspace --all-targets first with the same Rust toolchain.
Run this script from any directory. It uses temporary files and a loopback TCP server.
It does not start SDR hardware or contact an external broker/AI service.
Some probes compile verbatim private production functions/source into a harness;
others call public APIs from the compiled workspace libraries. See REPORT.md.
"""
from pathlib import Path
import subprocess, tempfile, hashlib, json, os

root = Path(__file__).resolve().parents[2]
work = Path(tempfile.mkdtemp(prefix='ez-sdr-logic-audit-'))
def extract(path, signature):
    source = (root / path).read_text()
    start = source.index(signature)
    brace = source.index('{', start)
    depth = 1
    end = brace + 1
    while depth:
        if source[end] == '{': depth += 1
        if source[end] == '}': depth -= 1
        end += 1
    return source[start:end]

mapper = extract('ez-gui/src/app.rs', 'fn map_daemon_aircraft(')
ai = extract('ez-gui/src/ai_panel.rs', 'fn execute_tool_call(')
shared = extract('ez-gui/src/test_helpers.rs', 'pub fn make_shared_state(')
squelch = extract('ez-gui/src/radio_ui.rs', 'pub(crate) fn set_power_squelch_level(')
sync_squelch = extract('ez-gui/src/radio_ui.rs', 'pub(crate) fn sync_squelch_level(')
packet_source=(root/'ez-daemon/src/pipelines/packet.rs').read_text()
(work/'packet.rs').write_text('\n'.join(line for line in packet_source.splitlines() if not line.startswith('//!')))
code = r'''
#![allow(dead_code, unused_imports)]
pub use ez_gui::*;
pub use ez_daemon::{bus, broadcast};
use std::sync::{Arc, Mutex};
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use ez_gui::app::{FreqMemEntry, SharedState};
use ez_gui::bookmarks::BookmarkDb;
use ez_gui::config::AppConfig;
use ez_gui::scheduler::Scheduler;
use ez_gui::source_manager::{SourceManager, SourceMode};
use ez_gui::spectrum::SpectrumAnalyzer;
use ez_gui::tle_engine::TleEngine;
mod local_probe {
    include!("__LOCAL_SOURCE__");
    pub fn inject(d: &mut AdsBDecoder, msg: [u8;14], reliable: bool) {
        let icao = ((msg[1] as u32)<<16)|((msg[2] as u32)<<8)|msg[3] as u32;
        d.process_decoded(icao, msg[0]>>3, msg, 0.5, reliable, 0, dump1090::demod::ScoreRank::Df17Known);
    }
}
mod packet_probe {
    include!("__PACKET_PATH__");
    pub fn inject(d: &mut PacketPipeline, mm: &dump1090::demod::ModesMessage) {d.merge_message(mm);}
    pub fn snapshot(d: &PacketPipeline) -> Vec<ez_proto::AircraftTelemetry> {d.aircraft.values().cloned().collect()}
}
struct Mapper;
impl Mapper { __MAPPER__ }
struct Ai { shared: Arc<Mutex<SharedState>> }
impl Ai { __AI__ }
mod radio_ui {
    pub use ez_gui::radio_ui::*;
    use ez_gui::app::SharedState;
    __SQUELCH__
    __SYNC_SQUELCH__
}
__SHARED__
fn parity(mut msg: [u8;14]) -> [u8;14] {
    let crc=dump1090::crc::crc24(&msg[..11]);
    msg[11]=(crc>>16) as u8;msg[12]=(crc>>8)as u8;msg[13]=crc as u8;
    assert!(dump1090::crc::check_crc(&msg)); msg
}
fn item() -> ez_proto::AircraftTelemetry {
    ez_proto::AircraftTelemetry { icao:0xabcdef,callsign:Some("AUDIT".into()),altitude_ft:Some(-500),lat:Some(51.0),lon:Some(0.0),ground_speed_kt:None,track_deg:None,vertical_rate_fpm:Some(640),msg_count:4,last_seen_ms:1 }
}
fn emit(id: &str, value: serde_json::Value) { println!("AUDIT {}",serde_json::json!({"probe":id,"observed":value})); }
fn main() {
    let output=std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let mut filter=dump1090::icao_filter::IcaoFilter::new();
    let known=0xabcdef;filter.add(known);
    let collision=(1..0xffffff).find(|&x| x!=known && filter.contains(x)).unwrap();
    let mut ap=[0u8;14];ap[0]=4<<3;
    let crc=dump1090::crc::crc24(&ap[..4])^collision;
    ap[4]=(crc>>16)as u8;ap[5]=(crc>>8)as u8;ap[6]=crc as u8;
    let mut mm=dump1090::demod::ModesMessage::default();
    let accepted=dump1090::demod::decode_mode_s_message(&mut mm,&ap,&mut filter,false,0).is_ok();
    assert!(accepted);assert_eq!(mm.addr,collision);
    emit("icao_collision_ap",serde_json::json!({"learned":format!("{known:06X}"),"unlearned_accepted":format!("{collision:06X}"),"accepted":accepted}));
    let mut filter=dump1090::icao_filter::IcaoFilter::new();
    for _ in 0..8192 {filter.add(known);}
    assert!(!filter.contains(known));
    emit("icao_duplicate_rotation",serde_json::json!({"duplicate_adds":8192,"known_after":filter.contains(known)}));

    let mut d=local_probe::AdsBDecoder::new();
    let mut vr=[0u8;14];vr[0]=0x8d;vr[1]=0xab;vr[2]=0xcd;vr[3]=0xef;vr[4]=(19<<3)|1;vr[8]=1;vr[9]=0x60;
    local_probe::inject(&mut d,parity(vr),true);
    let before=d.get_aircraft()[0].vertical_rate;
    let mut status=vr;status[4]=(28<<3)|1;status[5..11].fill(0);
    local_probe::inject(&mut d,parity(status),true);
    let after=d.get_aircraft()[0].vertical_rate;
    assert_eq!(after,Some(1));assert_ne!(before,after);
    emit("status_corrupts_vertical_rate",serde_json::json!({"before_fpm":before,"after_no_emergency_fpm":after}));
    let mut neg=vr;neg[4]=11<<3;neg[5]=0x05;neg[6]=0x80;
    let altitude=dump1090::mode_s::decode_mode_s_message(&parity(neg)).unwrap().altitude;
    assert_eq!(altitude,Some(0));
    emit("negative_altitude",serde_json::json!({"encoded_ft":-500,"decoded_ft":altitude}));
    let mut heading=vr;heading[4]=(19<<3)|3;heading[5]=0x05;heading[6]=0x00;
    let mut h=local_probe::AdsBDecoder::new();local_probe::inject(&mut h,parity(heading),true);
    let ac=&h.get_aircraft()[0];assert_eq!(ac.heading,90);assert!(!ac.has_heading);
    emit("airspeed_heading_validity",serde_json::json!({"heading":ac.heading,"has_heading":ac.has_heading}));
    let mut df18=neg;df18[0]=(18<<3)|1;
    let mut namespace=local_probe::AdsBDecoder::new();
    local_probe::inject(&mut namespace,parity(neg),true);
    local_probe::inject(&mut namespace,parity(df18),true);
    assert_eq!(namespace.get_aircraft().len(),1);
    emit("df18_identity_merge",serde_json::json!({"df17_and_non_icao_df18_cf1_entries":namespace.get_aircraft().len(),"merged_id":namespace.get_aircraft()[0].icao}));

    let bus=bus::SampleBus::new();let tap=bus.subscribe(4,bus::OverflowPolicy::DropIncoming);
    let mut packet=packet_probe::PacketPipeline::new(tap,2400000);
    let mut allcall=[0u8;14];allcall[0]=11<<3;allcall[1]=0xab;allcall[2]=0xcd;allcall[3]=0xef;
    let crc=dump1090::crc::crc24(&allcall[..4])^1;allcall[4]=(crc>>16)as u8;allcall[5]=(crc>>8)as u8;allcall[6]=crc as u8;
    let mut f=dump1090::icao_filter::IcaoFilter::new();f.add(0xabcdef);let mut mm=dump1090::demod::ModesMessage::default();
    let accepted=dump1090::demod::decode_mode_s_message(&mut mm,&allcall,&mut f,false,0).is_ok();
    assert!(!accepted);
    let semantic_accepted=dump1090::mode_s::decode_mode_s_message(&allcall).is_some();assert!(!semantic_accepted);
    emit("df11_iid_contract",serde_json::json!({"demod_accepted":accepted,"iid":1,"semantic_accepted":semantic_accepted,"previously_learned":true,"crc_repair_limit":0}));
    let mut local=local_probe::AdsBDecoder::new();
    for hex in ["8D40621D58C382D690C8AC2863A7","8D40621D58C386435CC412692AD6"] {
        let mut msg=[0u8;14];for i in 0..14 {msg[i]=u8::from_str_radix(&hex[i*2..i*2+2],16).unwrap();}
        msg[4]=(20<<3)|(msg[4]&7);msg=parity(msg);
        let mut mm=dump1090::demod::ModesMessage::default();mm.msg=msg;mm.msgtype=17;mm.addr=0x40621d;mm.msgbits=112;
        local_probe::inject(&mut local,msg,true);packet_probe::inject(&mut packet,&mm);
    }
    let local_position=local.get_aircraft()[0].has_position;
    let daemon_position=packet_probe::snapshot(&packet).iter().any(|a| a.lat.is_some());
    assert!(local_position);assert!(!daemon_position);
    emit("geometric_cpr_omission",serde_json::json!({"type_code":20,"local_position":local_position,"daemon_position":daemon_position}));

    let mut no_pos=item();no_pos.lat=None;no_pos.lon=None;
    assert!(Mapper::map_daemon_aircraft(no_pos).is_none());
    let mapped=Mapper::map_daemon_aircraft(item()).unwrap();
    assert!(mapped.has_heading&&mapped.reliable&&mapped.vertical_rate.is_none());
    emit("daemon_adapter",serde_json::json!({"no_position_dropped":true,"missing_track_has_heading":mapped.has_heading,"quality":mapped.score,"reliable":mapped.reliable,"vertical_rate":mapped.vertical_rate,"altitude":mapped.altitude,"reported_age_ms":1,"mapped_age_ms":mapped.seen.elapsed().as_millis()}));

    let samples=[90u8,110,127,128,140,160,180,200];
    let mut writer=satellite::recorder::Cf32StreamWriter::start(output.to_str().unwrap(),Some("ROUNDTRIP".into()),2400000,1090000000,0.0,0.0).unwrap();
    let path=writer.path().to_owned();writer.write(&samples);writer.stop().unwrap();
    let mut source=SourceManager::new();source.source_mode=SourceMode::Replay;source.replay_file=Some(path.to_string_lossy().into());source.replay_loop=false;source.start();
    let deadline=Instant::now()+Duration::from_secs(2);
    let replay=loop {if let Some(v)=source.recv_samples(){break v;}assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(2));};
    let position=source.replay_position;let size=source.replay_size;source.stop();
    assert_ne!(samples.to_vec(),replay);
    emit("cf32_roundtrip",serde_json::json!({"original":samples,"replayed":replay,"progress_bytes":position,"input_file_bytes":size}));
    let mut broken=satellite::recorder::Cf32StreamWriter::start(output.to_str().unwrap(),Some("missing/parent".into()),2400000,1090000000,0.0,0.0).unwrap();
    std::thread::sleep(Duration::from_millis(30));let claimed=broken.is_running();broken.write(&samples);
    let stopped=broken.stop().unwrap();assert!(claimed);assert_eq!(stopped.bytes_total,0);
    emit("cf32_failure_reporting",serde_json::json!({"start_ok":true,"is_running_after_worker_exit":claimed,"stop_ok":true,"bytes":stopped.bytes_total}));
    let mut overwrite = None;
    for _ in 0..3 {
        let mut first=satellite::recorder::Cf32StreamWriter::start(output.to_str().unwrap(),Some("COLLISION".into()),2400000,1090000000,0.0,0.0).unwrap();
        let path=first.path().to_owned();first.write(&[90,110,140,160]);first.stop().unwrap();let before=std::fs::metadata(&path).unwrap().len();
        let mut second=satellite::recorder::Cf32StreamWriter::start(output.to_str().unwrap(),Some("COLLISION".into()),2400000,1090000000,0.0,0.0).unwrap();
        let same_path=path==second.path();second.write(&[180,200]);second.stop().unwrap();
        if same_path {overwrite=Some((before,std::fs::metadata(&path).unwrap().len()));break;}
    }
    let (before,after)=overwrite.expect("two starts must fall in same second within three attempts");assert_eq!(before,16);assert_eq!(after,8);
    emit("cf32_overwrite",serde_json::json!({"same_path":true,"first_bytes":before,"remaining_bytes":after}));

    let shared=make_shared_state();let mut recorder=recorder_panel::RecorderPanel::new(shared.clone());
    let lock=shared.lock().unwrap();recorder.handle_daemon_recording(ez_proto::RecordingStatus{channel_id:1,active:true,path:None,bytes_written:10,duration_sec:1.0});
    assert!(recorder.recording);assert!(!lock.recording);drop(lock);
    emit("daemon_recorder_lock",serde_json::json!({"panel_recording":recorder.recording,"shared_recording":shared.lock().unwrap().recording}));
    recorder.recording=false;recorder.record_iq=true;recorder.record_audio=false;recorder.output_dir=output.join("iq").to_string_lossy().into();
    recorder.max_duration_mins=1;recorder.start_recording();assert!(recorder.recording);
    recorder.start_time=Some(Instant::now()-Duration::from_secs(61));recorder.write_samples(&samples);
    assert!(recorder.recording);let before=recorder.bytes_written;
    let ctx=egui::Context::default();let _=ctx.run_ui(egui::RawInput::default(),|ctx|{egui::CentralPanel::default().show(ctx,|ui|recorder.ui(ui));});
    assert!(!recorder.recording);
    emit("hidden_recorder_deadline",serde_json::json!({"duration_limit_s":60,"elapsed_s":61,"bytes_accepted_after_limit":before,"stopped_only_after_ui":true}));
    let mut ai=Ai{shared:shared.clone()};let claimed=ai.execute_tool_call("start_recording",&serde_json::json!({}));
    assert!(shared.lock().unwrap().recording);assert!(!recorder.recording);
    let gen=shared.lock().unwrap().source.stream_generation();
    ai.execute_tool_call("tune_frequency",&serde_json::json!({"hz":1090000000}));
    ai.execute_tool_call("start_adsb",&serde_json::json!({}));
    let s=shared.lock().unwrap();assert_eq!(gen,s.source.stream_generation());
    emit("ai_tool_actions",serde_json::json!({"result":claimed,"shared_recording":s.recording,"recorder_recording":recorder.recording,"source_generation_changed":gen!=s.source.stream_generation(),"source_running":matches!(s.source.status,source_manager::SourceStatus::Running),"adsb_claimed":s.adsb_running,"sample_rate":s.source.sample_rate_hz}));drop(s);

    let bus=ez_daemon::bus::SampleBus::new();let tap=bus.subscribe(4,ez_daemon::bus::OverflowPolicy::DropIncoming);
    let mut ch=ez_daemon::channelizer::Channelizer::new(tap,1090000000,2400000);let (_,out)=ch.add_channel(1090000000,2400000).unwrap();
    for start in [0,1000] {bus.publish(ez_daemon::bus::SampleBlock{start_sample:start,sample_rate_hz:2400000,center_freq_hz:1090000000,samples:Arc::from(vec![num_complex::Complex32::new(1.0,0.0);4])});assert!(ch.tick(Duration::ZERO));}
    let first=out.try_recv().unwrap();let second=out.try_recv().unwrap();assert_eq!(second.start_sample,4);
    emit("channelizer_erases_gap",serde_json::json!({"input_starts":[0,1000],"input_block_len":4,"output_starts":[first.start_sample,second.start_sample]}));

    let worker=demod::DemodWorker::new(8);
    for _ in 0..4 {assert!(worker.configure(demod::DemodConfig::default()));}
    let mut latest=demod::DemodConfig::default();latest.mode=sdr_panel::DemodMode::Am;
    let accepted=worker.configure(latest);assert!(!accepted);
    assert!(worker.push_iq(vec![num_complex::Complex32::new(0.5,0.0);8192]));
    let deadline=Instant::now()+Duration::from_secs(2);
    let frame=loop {if let Some(f)=worker.try_recv_frame(){break f;}assert!(Instant::now()<deadline);std::thread::sleep(Duration::from_millis(2));};
    assert_eq!(frame.mode,sdr_panel::DemodMode::Fm);
    emit("demod_config_not_coalesced",serde_json::json!({"latest_requested":"AM","latest_accepted":accepted,"frame_mode":frame.mode.label()}));
    drop(worker);

    let mut spectrum=SpectrumAnalyzer::new();spectrum.update_params(100000000,2048000);spectrum.set_avg_alpha(1.0);
    spectrum.vfo_freq_hz=Some(100000000);spectrum.vfo_bw_hz=12500;
    let mut bins=vec![-120.0;spectrum.fft_size()];let bin=bins.len()/2+200;bins[bin]=-10.0;
    spectrum.push_spectrum_frame(&ez_proto::SpectrumFrame{center_hz:100000000,sample_rate_hz:2048000,bins,timestamp_ms:1});
    let global=spectrum.peak_level();let vfo=spectrum.vfo_signal_level();
    let mut scanner=scanner::FrequencyScanner::new(shared.clone());scanner.start_hz=100000000;scanner.stop_hz=101000000;scanner.dwell_ms=0;scanner.threshold_db=-50.0;scanner.start();scanner.tick(global);
    assert!(vfo < -50.0);assert_eq!(scanner.hits.len(),1);assert_eq!(scanner.hits[0].freq_hz,100000000);
    emit("scanner_off_channel_hit",serde_json::json!({"wideband_peak_db":global,"selected_channel_db":vfo,"actual_peak_hz":spectrum.peak_freq_hz(),"logged_hit_hz":scanner.hits[0].freq_hz}));

    use std::io::{Read,Write};
    let mut server=rigctl::RigctlServer::new();server.set_enabled(true,0);
    for _ in 0..100 {if server.bound_port()!=0{break;}std::thread::sleep(Duration::from_millis(2));}
    let mut stream=std::net::TcpStream::connect(("127.0.0.1",server.bound_port())).unwrap();stream.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    stream.write_all(b"F 1").unwrap();std::thread::sleep(Duration::from_millis(80));stream.write_all(b"090000000\n").unwrap();
    std::thread::sleep(Duration::from_millis(30));let reqs=server.poll_requests();let mut buf=[0u8;64];let n=stream.read(&mut buf).unwrap();
    assert!(reqs.is_empty());emit("rigctl_fragmentation",serde_json::json!({"sent_command":"F 1090000000","delay_ms":80,"requests":reqs.len(),"reply":String::from_utf8_lossy(&buf[..n])}));drop(stream);server.stop();
}
'''.replace('__MAPPER__',mapper).replace('__AI__',ai).replace('__SHARED__',shared).replace('__SQUELCH__',squelch).replace('__SYNC_SQUELCH__',sync_squelch).replace('__PACKET_PATH__',str(work/'packet.rs')).replace('__LOCAL_SOURCE__',str(root/'ez-gui/src/adsb_decoder.rs'))
source=work/'probes.rs';source.write_text(code)
toolchain=Path(os.environ.get('EZ_AUDIT_RUST_BIN', str(Path.home()/'.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin')))
args=[str(toolchain/'rustc'),'--edition=2021',str(source),'-o',str(work/'probes'),'-L',f'dependency={root}/target/debug/deps','-L','native=/usr/local/lib']
for lib in ['ez_gui','ez_daemon','ez_proto','dump1090','serde_json','num_complex','egui','tracing']:
    files=list((root/'target/debug/deps').glob(f'lib{lib}-*.rlib'))
    args+=['--extern',f'{lib}={max(files,key=lambda p:p.stat().st_mtime)}']
for p in (root/'target/debug/build').glob('*/out'):args+=['-L',f'native={p}']
compiled=subprocess.run(args,text=True,capture_output=True)
(work/'compile.log').write_text(compiled.stdout+compiled.stderr)
if compiled.returncode:
    print(compiled.stderr);raise SystemExit(compiled.returncode)
result=subprocess.run([str(work/'probes'),str(work)],text=True,capture_output=True)
(work/'output.log').write_text(result.stdout+result.stderr)
rows=[json.loads(line[6:]) for line in result.stdout.splitlines() if line.startswith('AUDIT ')]
manifest={'source_commit':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),'rustc':subprocess.check_output([str(toolchain/'rustc'),'--version'],text=True).strip(),'exit_code':result.returncode,'probes':rows,'temporary_work_dir':str(work),'source_hashes':{str(p):hashlib.sha256((root/p).read_bytes()).hexdigest() for p in ['ez-gui/src/app.rs','ez-gui/src/ai_panel.rs','ez-gui/src/adsb_decoder.rs','ez-gui/src/test_helpers.rs','ez-gui/src/recorder_panel.rs','ez-gui/src/source_manager.rs','ez-gui/src/satellite/recorder.rs','ez-gui/src/scanner.rs','ez-gui/src/spectrum.rs','ez-gui/src/demod.rs','ez-gui/src/rigctl.rs','ez-daemon/src/channelizer.rs','ez-daemon/src/pipelines/packet.rs','dump1090/src/icao_filter.rs','dump1090/src/mode_s.rs','dump1090/src/demod.rs']}}
(work/'evidence.json').write_text(json.dumps(manifest,indent=2))
print(result.stdout);print(result.stderr);print('WORKDIR',work)
raise SystemExit(result.returncode)
