import json, os, re, subprocess, time
from pathlib import Path
line = next(x for x in Path('/proc/self/cgroup').read_text().splitlines() if x.startswith('0::'))
cgroup = Path('/sys/fs/cgroup') / line.split('::', 1)[1].lstrip('/')
cpu_max = (cgroup / 'cpu.max').read_text().strip()
assert cpu_max == '10000 100000', cpu_max
assert os.getpriority(os.PRIO_PROCESS, 0) == 19
resource = {'cgroup': str(cgroup), 'cpu_max': cpu_max, 'nice': 19, 'build_jobs': 1, 'test_threads': 1}
commands = [('rf-selection', ['cargo', 'test', '-p', 'ez-daemon', '--no-default-features', '-j', '1', '--lib', 'rf_selection', '--', '--nocapture', '--test-threads=1']), ('raw-audio', ['cargo', 'test', '-p', 'ez-gui', '--no-default-features', '-j', '1', '--lib', 'raw_audio::tests::', '--', '--nocapture', '--test-threads=1']), ('playback', ['cargo', 'test', '-p', 'ez-gui', '--no-default-features', '-j', '1', '--lib', 'audio_resampler::tests::', '--', '--nocapture', '--test-threads=1']), ('detected-audio', ['cargo', 'test', '-p', 'ez-gui', '--no-default-features', '-j', '1', '--lib', 'detected_antialias', '--', '--nocapture', '--test-threads=1']), ('daemon-clippy', ['cargo', 'clippy', '-p', 'ez-daemon', '--all-targets', '-j', '1', '--', '-D', 'warnings']), ('gui-clippy', ['cargo', 'clippy', '-p', 'ez-gui', '--all-targets', '-j', '1', '--', '-D', 'warnings']), ('gui-build', ['cargo', 'build', '-p', 'ez-gui', '-j', '1']), ('format', ['cargo', 'fmt', '--all', '--', '--check'])]
results=[]
for name, command in commands[1:]:
    print('Running '+name+' under 10% CPU cap.', flush=True)
    log = Path('/tmp/ez-sdr-r19-foundation-final-'+name+'.log')
    start=time.monotonic()
    with log.open('wb') as output:
        result=subprocess.run(command, stdout=output, stderr=subprocess.STDOUT)
    content=log.read_text()
    record={'name':name,'command':command,'exit_code':result.returncode,'seconds':time.monotonic()-start,'raw_log':str(log)}
    if 'test' in command and result.returncode == 0:
        harness=re.findall(r'test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored',content)
        assert harness, content
        record['test_harnesses']=[{'passed':int(a),'failed':int(b),'ignored':int(c)} for a,b,c in harness]
    results.append(record)
    print(name+': exit '+str(result.returncode)+'; '+str(round(record['seconds'],2))+' seconds', flush=True)
    if result.returncode:
        print(content[-4500:], flush=True)
    payload={'resource_policy':resource,'commands':results,'cpu_stat':(cgroup/'cpu.stat').read_text()}
    Path('/tmp/ez-sdr-r19-foundation-final-verification-results.json').write_text(json.dumps(payload,indent=2)+'\n')
    if result.returncode:
        raise SystemExit(result.returncode)
print('Shared audio/RF foundation checks complete.', flush=True)
