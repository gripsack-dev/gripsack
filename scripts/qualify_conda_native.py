import hashlib, json, os, platform, shutil, signal, subprocess, tempfile
from pathlib import Path

core=Path(os.environ.get('GRIP_RUNTIME_BIN','/home/tarek/workspace/gripsack-workspace-buildkit/target/debug/grip')).resolve()
helper=Path(os.environ.get('HELPER','/home/tarek/workspace/gripsack-workspace-buildkit/tools/conda-helper/target/release/gripsack-conda')).resolve()
root=Path(tempfile.mkdtemp(prefix='gripsack conda native '+('x'*50)+' ')).resolve()
repo=root/'repo'; repo.mkdir(); home=root/'home'; home.mkdir(); state=root/'state'; state.mkdir(mode=0o700)
osname='macos' if platform.system()=='Darwin' else 'linux'
arch={'arm64':'aarch64','aarch64':'aarch64','x86_64':'x86_64'}[platform.machine()]
target={'os':osname,'arch':arch,'abi':'darwin' if osname=='macos' else 'gnu'}
env={'PATH':'/usr/bin:/bin:/usr/sbin:/sbin','HOME':str(home),'GRIPSACK_HOME':str(state),'GRIPSACK_CONDA_HELPER':str(helper),'LANG':'C.UTF-8','SHELL':'/bin/sh'}
if os.environ.get('GRIPSACK_DENO'): env['GRIPSACK_DENO']=os.environ['GRIPSACK_DENO']
elif osname=='linux': env['GRIPSACK_DENO']='/tmp/gripsack-workspace-smoke-deno'
seed=Path('/tmp/gripsack-conda-core-6wlm2l6b/state/store')
if osname=='linux' and seed.is_dir():
    store=state/'store'; store.mkdir()
    for archive in seed.glob('*-conda-archive'):
        shutil.copytree(archive,store/archive.name,symlinks=True)
print('CONDA_NATIVE_FIXTURE='+str(root),flush=True)
code='import json,sys,numpy,wheel,_ssl,_sqlite3,zlib; assert sys.dont_write_bytecode; assert sys.argv[1:]==["","two words"]; print(json.dumps({"prefix":sys.prefix,"version":sys.version.split()[0],"numpy":numpy.__version__}))'
def invoke(*args,expected=0,stdin=None,deadline=600):
    child=subprocess.Popen([str(core),*args],cwd=repo,env=env,stdin=subprocess.PIPE if stdin is not None else subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,start_new_session=True)
    try: out,err=child.communicate(stdin,timeout=deadline)
    except BaseException:
        os.killpg(child.pid,signal.SIGKILL); child.communicate(); raise
    print('COMMAND='+ ' '.join(args)+' STATUS='+str(child.returncode),flush=True)
    if child.returncode!=expected: raise RuntimeError(out[-10000:]+err[-10000:])
    return out

def approve():
    data=json.loads(invoke('trust','inspect','--json'))
    invoke('trust','add','--bundle',data['bundle_digest'],'--policy',data['policy_digest'])

def declare(python):
    packages={'python':python,'numpy':'*','wheel':'*'}
    if osname=='linux': packages['xorg-libx11']='*'
    (repo/'gripsack.ts').write_text('import {defineWorkspace,workspace,pkg,provider,conda,environment,profile,task,exec,packageCommand,lit} from "@gripsack/core";\n'
      +'const target='+json.dumps(target)+';\n'
      +'export default defineWorkspace(()=>workspace({outputs:[\n'
      +'pkg("pyenv",{producer:provider(conda.environment({channels:["conda-forge"],packages:'+json.dumps(packages)+'})),commands:{python:"bin/python3",wheel:"bin/wheel"},target,layout:{kind:"prefix_materialized"}}),\n'
      +'environment("dev",{packages:["pyenv"],target}), profile("personal",{environment:"dev"}),\n'
      +'task("direct",{steps:[exec(packageCommand("pyenv","python")).arg(lit("-c")).arg(lit('+json.dumps(code)+')).arg(lit("")).arg(lit("two words")).build()]})\n]}));\n')
    approve()

def snapshot(prefix):
    digest=hashlib.sha256()
    for path in sorted(prefix.rglob('*')):
        stat=path.lstat(); digest.update(str(path.relative_to(prefix)).encode()); digest.update(str(stat.st_mode).encode())
        if path.is_symlink(): digest.update(os.readlink(path).encode())
        elif path.is_file():
            with path.open('rb') as stream: digest.update(hashlib.file_digest(stream,'sha256').digest())
    return digest.hexdigest()

def execute(argv):
    child=subprocess.Popen(argv,cwd=repo,env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True,start_new_session=True)
    try: out,err=child.communicate(timeout=120)
    except BaseException:
        os.killpg(child.pid,signal.SIGKILL); child.communicate(); raise
    if child.returncode: raise RuntimeError(out+err)
    return out

declare('3.12.*')
invoke('update','pyenv'); approve()
lock=(repo/'gripsack.lock').read_bytes()
first=json.loads(invoke('build','pyenv','--json'))['outputs'][0]
prefix=Path(first['path']); assert len(str(prefix))>125 and ' ' in str(prefix)
before=snapshot(prefix)
env['GRIPSACK_CONDA_HELPER']=str(root/'absent-helper')
env['PIXI_HOME']=str(root/'removed-private-pixi')
result=json.loads(invoke('run','--env','dev','--','python','-c',code,'','two words'))
assert result['prefix']==str(prefix) and result['version'].startswith('3.12.'),result
assert json.loads(invoke('task','direct'))['prefix']==str(prefix)
assert 'wheel' in invoke('run','--env','dev','--','wheel','version').lower()
shell=invoke('shell','dev',stdin="python -c 'import sys,numpy; assert sys.dont_write_bytecode; print(\"CONDA_SHELL_OK\")'\nexit\n")
assert 'CONDA_SHELL_OK' in shell,shell
invoke('apply','personal')
wrappers=list((state/'store').glob('*-workspace-files/commands/python'))
assert len(wrappers)==1,wrappers
old_wrapper=wrappers[0]
assert json.loads(execute([str(old_wrapper),'-c',code,'','two words']))['prefix']==str(prefix)
assert snapshot(prefix)==before,'native consumers changed the published prefix'
assert (repo/'gripsack.lock').read_bytes()==lock,'frozen consumer rewrote lock'
assert not (state/'buildkit').exists(),'native Conda started a builder'
assert not Path(env['PIXI_HOME']).exists(),'native Conda required private Pixi state'
print('CONDA_NATIVE_RUN_TASK_SHELL_PROFILE_LONG_SPACED_UNCHANGED=passed',flush=True)

env['GRIPSACK_CONDA_HELPER']=str(helper)
declare('3.13.*'); invoke('update','pyenv'); approve()
second=json.loads(invoke('build','pyenv','--json'))['outputs'][0]
assert second['path']!=first['path']
invoke('apply','personal')
env['GRIPSACK_CONDA_HELPER']=str(root/'absent-helper')
invoke('gc')
assert json.loads(execute([str(old_wrapper),'-c',code,'','two words']))['version'].startswith('3.12.')
current=json.loads(invoke('run','--env','dev','--','python','-c',code,'','two words'))
assert current['version'].startswith('3.13.') and current['prefix']==second['path'],current
assert snapshot(prefix)==before,'update/GC changed the retained old prefix'
print('CONDA_COHERENT_UPDATE_RETAINED_PROFILE_AND_GC=passed',flush=True)
print('CONDA_NATIVE_QUALIFIED_ROOT='+str(root),flush=True)
