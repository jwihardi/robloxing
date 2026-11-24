using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Threading;
using System.Web.Script.Serialization;

// Controlled stdio provider for the Windows job-object and desktop lifecycle probe.
class FixtureProvider {
    static readonly JavaScriptSerializer Json = new JavaScriptSerializer();
    static readonly string ThreadId = Guid.NewGuid().ToString();
    static string session, turn, task;
    static bool worker;
    static string Dir { get { return Environment.GetEnvironmentVariable("WORKSHOP_PROBE_DIR"); } }
    static void Send(object message) { Console.WriteLine(Json.Serialize(message)); Console.Out.Flush(); }
    static void Done(string text) {
        Send(new { method="item/agentMessage/delta", @params=new {threadId=session,turnId=turn,itemId="reply",delta=text} });
        Send(new { method="item/completed", @params=new {threadId=session,turnId=turn,item=new {type="agentMessage",id="reply",text=text}} });
        Send(new { method="thread/tokenUsage/updated", @params=new {threadId=session,tokenUsage=new {total=new {totalTokens=100}}} });
        Send(new { method="turn/completed", @params=new {threadId=session,turn=new {id=turn,status="completed"}} });
    }
    static object Assignment(string key, params string[] dependencies) {
        return new {key=key,title="Write "+key,deliverable="Write "+key+".txt",acceptance="File contains its name",files=new[]{File.Exists(Path.Combine(Dir,"overlap"))?".":key+".txt"},depends_on=dependencies};
    }
    static void Worker(string prompt) {
        task = prompt.Split('\n')[0].Substring("Write ".Length).Trim();
        if (task == "guide" && File.ReadAllText("left.txt").Trim() != "left") throw new InvalidDataException("Missing first integration");
        if (task == "summary" && (File.ReadAllText("guide.txt").Trim() != "guide" || File.ReadAllText("right.txt").Trim() != "right")) throw new InvalidDataException("Missing joined integrations");
        int pid=Process.GetCurrentProcess().Id;
        using(var child=Process.Start(new ProcessStartInfo(Process.GetCurrentProcess().MainModule.FileName,"sleeper"){UseShellExecute=false,CreateNoWindow=true})){
            string marker=Path.Combine(Dir,task+".ready");
            File.WriteAllText(marker,pid+","+child.Id);
            while(File.Exists(Path.Combine(Dir,task+".hold"))) Thread.Sleep(40);
            File.WriteAllText(task+".txt",task+"\n");
            Done("Wrote "+task+".txt");
            // The harness tears down the complete job when the worker turn ends.
        }
    }
    static void Main(string[] args) {
        if(args.Length > 0 && (args[0]=="auth" || args[0]=="login" || args[0]=="logout")){
            bool claude=args[0]=="auth";
            string file=Path.Combine(Dir,claude ? "claude.session" : "codex.session");
            string op=args[args.Length-1];
            if(op=="--json"){Send(new {loggedIn=File.Exists(file)});return;}
            if(op=="status"){Console.WriteLine(File.Exists(file) ? "Logged in using ChatGPT" : "Not logged in");return;}
            if(op=="login"){File.WriteAllText(file,"fixture");return;}
            if(op=="logout"){File.Delete(file);return;}
        }
        if(args.Length > 1 && args[0]=="plugin"){
            string file=Path.Combine(Dir,"sample.plugin");
            var plugin=new {pluginId="sample@fixture",name="Sample plugin",marketplaceName="fixture"};
            if(args[1]=="list"){Send(new {installed=File.Exists(file) ? new object[]{plugin} : new object[0],available=new[]{plugin}});return;}
            if(args[1]=="add" || args[1]=="install"){File.WriteAllText(file,args[2]);return;}
            if(args[1]=="remove" || args[1]=="uninstall"){File.Delete(file);return;}
        }
        if(args.Length==1 && args[0]=="check"){ if(File.Exists(Path.Combine(Dir,"fail-check"))){Console.WriteLine("fixture verification failed");Environment.Exit(9);} return;}
        if(args.Length==1 && args[0]=="sleeper"){Thread.Sleep(Timeout.Infinite);return;}
        string line;
        while((line=Console.ReadLine())!=null){
            var msg=Json.Deserialize<Dictionary<string,object>>(line);
            if(!msg.ContainsKey("method")){
                if(Convert.ToString(msg["id"])=="assignment") Done("Worker batch returned.");
                continue;
            }
            string method=(string)msg["method"];
            if(method=="initialized") continue;
            var p=(Dictionary<string,object>)msg["params"];
            var id=msg["id"];
            if(method=="initialize") Send(new {id=id,result=new {}});
            if(method=="config/read") Send(new {id=id,result=new {config=new {model="fixture-fast",model_reasoning_effort="low"}}});
            if(method=="model/list"){
                var low=new {reasoningEffort="low"}; var high=new {reasoningEffort="high"};
                Send(new {id=id,result=new {data=new[]{new {model="fixture-fast",defaultReasoningEffort="low",supportedReasoningEfforts=new[]{low},isDefault=true},
                    new {model="fixture-deep",defaultReasoningEffort="high",supportedReasoningEfforts=new[]{low,high},isDefault=false}}}});
            }
            if(method=="thread/start" || method=="thread/resume"){
                session=p.ContainsKey("threadId")?(string)p["threadId"]:ThreadId;
                worker=((string)p["developerInstructions"]).StartsWith("You are a Workshop Codex worker");
                Send(new {id=id,result=new {thread=new {id=session}}});
            }
            if(method=="thread/loaded/list") Send(new {id=id,result=new {data=new[]{session}}});
            if(method=="turn/start"){
                if(File.Exists(Path.Combine(Dir,"record-inference")))
                    File.WriteAllText(Path.Combine(Dir,(worker?"worker-":"lead-")+Guid.NewGuid()+".json"),Json.Serialize(p));
                turn=Guid.NewGuid().ToString();
                Send(new {id=id,result=new {turn=new {id=turn}}});
                var first=(Dictionary<string,object>)((System.Collections.IList)p["input"])[0];
                if(worker){Worker((string)first["text"]);continue;}
                var tasks=new List<object>{Assignment("left"),Assignment("right")};
                if(File.Exists(Path.Combine(Dir,"mixed"))){
                    tasks.Add(Assignment("guide","left"));
                    tasks.Add(Assignment("summary","guide","right"));
                }
                Send(new {id="assignment",method="item/tool/call",@params=new {threadId=session,turnId=turn,callId="call",tool="workshop_delegate",arguments=new {tasks=tasks}}});
            }
            if(method=="turn/interrupt"){
                Send(new {id=id,result=new {}});
                Send(new {method="turn/completed",@params=new {threadId=session,turn=new {id=turn,status="interrupted"}}});
            }
        }
    }
}
