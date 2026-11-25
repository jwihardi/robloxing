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
    static string session, turn, task, studioKind;
    static bool worker, studio, claudeStream;
    static string Dir { get { return Environment.GetEnvironmentVariable("WORKSHOP_PROBE_DIR"); } }
    static void Send(object message) { Console.WriteLine(Json.Serialize(message)); Console.Out.Flush(); }
    static void Done(string text) {
        if (claudeStream) {
            var usage = new {input_tokens=24,output_tokens=3};
            Send(new {type="stream_event", @event=new {type="content_block_delta",delta=new {type="text_delta",text=text}}});
            Send(new {type="assistant", message=new {model="fixture",usage=usage,content=new[]{new {type="text",text=text}}}});
            Send(new {type="result",subtype="success",is_error=false,result=text,usage=usage,modelUsage=new {fixture=new {contextWindow=100}}});
            return;
        }
        Send(new { method="item/agentMessage/delta", @params=new {threadId=session,turnId=turn,itemId="reply",delta=text} });
        Send(new { method="item/completed", @params=new {threadId=session,turnId=turn,item=new {type="agentMessage",id="reply",text=text}} });
        Send(new { method="thread/tokenUsage/updated", @params=new {threadId=session,tokenUsage=new {total=new {totalTokens=100}}} });
        Send(new { method="turn/completed", @params=new {threadId=session,turn=new {id=turn,status="completed"}} });
    }
    static object Assignment(string key, params string[] dependencies) {
        return new {key=key,title="Write "+key,deliverable="Write "+key+".txt",acceptance="File contains its name",files=new[]{File.Exists(Path.Combine(Dir,"overlap"))?".":key+".txt"},depends_on=dependencies};
    }
    static bool Mode(string name) { return File.Exists(Path.Combine(Dir, name)); }
    // Leave the input pipe live while a lead or a worker waits for a correction.
    static bool Delivery(string text) {
        if (!Mode("delivery-mode")) return false;
        if (worker && text.StartsWith("Write left\n")) {
            File.WriteAllText(Path.Combine(Dir, "waiting-worker"), "left");
            return true;
        }
        if (worker && text == "Use the blue version") {
            File.WriteAllText("left.txt", "blue\n");
            Done("Wrote the blue version");
            return true;
        }
        if (!worker) {
            File.AppendAllText(Path.Combine(Dir, "delivery.log"), text.Split('\n')[0] + "\n");
            if (text == "Wait for a correction") return true;
            if (text.StartsWith("Follow up") || text.StartsWith("Actually ")) {
                Done("Received " + text); return true;
            }
        }
        return false;
    }
    static void Delegate(List<object> tasks) {
        Send(new {id="assignment",method="item/tool/call",@params=new {threadId=session,turnId=turn,callId="batch",tool="workshop_delegate",arguments=new {tasks=tasks}}});
    }
    static object Audit(params string[] dependencies) {
        return new {key="audit",title="Write audit",deliverable="Independently inspect both files",acceptance="Both contain their own name",files=new string[0],depends_on=dependencies};
    }
    static void Report(string status, string summary) {
        Send(new {id="report",method="item/tool/call",@params=new {threadId=session,turnId=turn,callId="report",tool="workshop_report_result",arguments=new {status=status,summary=summary}}});
    }
    static void Worker(string prompt) {
        task = prompt.Split('\n')[0].Substring("Write ".Length).Trim();
        if (Mode("autonomous")) {
            if (!prompt.Contains("Deliver a release bundle")) throw new InvalidDataException("Missing creator goal");
            if (task == "audit") {
                if (!prompt.Contains("REPORT-LEFT-42")) throw new InvalidDataException("Missing prerequisite handoff");
                bool passed = File.ReadAllText("right.txt").Trim() == "right";
                Report(passed ? "complete" : "needs_changes", passed ? "Joined outputs pass acceptance" : "right.txt contains rigth; expected right");
                return;
            }
            if (task == "fix-right") {File.WriteAllText("right.txt","right\n"); Done("Corrected spelling"); return;}
            if (task == "publish") {
                if (!prompt.Contains("Joined outputs pass acceptance")) throw new InvalidDataException("Reviewer evidence missing");
                File.WriteAllText("publish.txt", "release ready\n"); Done("Release verified"); return;
            }
            File.WriteAllText(task+".txt",(task=="right" ? "rigth" : task)+"\n");
            Report("complete", task=="left" ? "REPORT-LEFT-42" : "First version of right.txt"); return;
        }
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
    static void Claude(string[] args) {
        claudeStream = true;
        int instructions = Array.IndexOf(args,"--append-system-prompt");
        worker = instructions >= 0 && args[instructions+1].StartsWith("You are a Workshop worker");
        int resume = Array.IndexOf(args,"--resume");
        session = resume >= 0 ? args[resume+1] : ThreadId;
        string line;
        while ((line=Console.ReadLine())!=null) {
            var frame = Json.Deserialize<Dictionary<string,object>>(line);
            switch ((string)frame["type"]) {
                case "control_request":
                    if (Convert.ToString(frame["request_id"]) == "workshop-steer" && Mode("delivery-mode")) {
                        Send(new {type="result",subtype="error_during_execution",is_error=true,terminal_reason="aborted_by_interrupt"});
                        break;
                    }
                    Send(new {type="control_response",response=new {subtype="success",request_id=frame["request_id"],response=new {models=new[]{new {value="default",supportedEffortLevels=new[]{"low","high"}}}}}});
                    break;
                case "user":
                    Send(new {type="system",subtype="init",session_id=session});
                    var msg=(Dictionary<string,object>)frame["message"];
                    var blocks=(System.Collections.ArrayList)msg["content"];
                    var first=(Dictionary<string,object>)blocks[0];
                    if (Delivery((string)first["text"])) break;
                    if (worker) { Worker((string)first["text"]); break; }
                    var tasks = new[]{Assignment("left"),Assignment("right")};
                    Send(new {type="control_request",request_id="batch", request=new {subtype="mcp_message",server_name="workshop",message=new {jsonrpc="2.0",id=4,method="tools/call",@params=new {name="workshop_delegate",arguments=new {tasks=tasks}}}}});
                    break;
                case "control_response":
                    if (!line.Contains("mcp_response")) throw new InvalidDataException("Missing MCP result");
                    if (Mode("delivery-mode")) File.WriteAllText(Path.Combine(Dir,"lead-feedback"),line);
                    Done("Worker batch returned.");
                    break;
            }
        }
    }
    static void Main(string[] args) {
        if (args.Length == 3 && args[0] == "models" && args[1] == "-o" && args[2] == "json") {
            Send(new[] {new {modelId="gemini-fixture"}});
            return;
        }
        if (Environment.GetEnvironmentVariable("GIT_DIR") != null && Mode("check-git-context"))
            throw new InvalidDataException("Provider or check inherited the caller's repository");
        if(args.Length>0 && args[0]=="-p") { Claude(args); return; }
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
            var official=new {pluginId="calendar@openai-curated-remote",name="Calendar",marketplaceName="openai-curated-remote",description="Plan events"};
            var sculpt=new {pluginId="image-to-sculpt@image-to-sculpt",name="Image to Sculpt",marketplaceName="image-to-sculpt"};
            string market = Path.Combine(Dir,"sculpt.market");
            if(args[1]=="marketplace" && args[2]=="list"){Send(File.Exists(market) ? new object[]{new {name="image-to-sculpt",source="github",repo="gokayfem/image-to-sculpt"}} : new object[0]);return;}
            if(args[1]=="marketplace" && args[2]=="add" && args[3]=="gokayfem/image-to-sculpt"){File.WriteAllText(market,args[3]);return;}
            if(args[1]=="install" && args[2]==sculpt.pluginId && !File.Exists(market)){Environment.Exit(1);}
            if(args[1]=="list"){Send(new {installed=File.Exists(file) ? new object[]{File.ReadAllText(file)==sculpt.pluginId ? (object)sculpt : plugin} : new object[0],available=new object[]{plugin,official}});return;}
            if(args[1]=="add" || args[1]=="install"){File.WriteAllText(file,args[2]);return;}
            if(args[1]=="remove" || args[1]=="uninstall"){File.Delete(file);return;}
        }
        if(args.Length==1 && args[0]=="joined-check"){
            File.AppendAllText(Path.Combine(Dir,"joins.log"),"check\n");
            foreach(string key in new[]{"left","right","guide","summary"}) if(!File.Exists(key+".txt")) {Console.WriteLine("Missing "+key); Environment.Exit(8);}
            return;
        }
        if(args.Length==1 && args[0]=="content-check"){
            if(Mode("check.hold")) { File.WriteAllText(Path.Combine(Dir,"check.ready"),Process.GetCurrentProcess().Id.ToString()); Thread.Sleep(Timeout.Infinite); }
            if(File.ReadAllText("answer.txt").Trim()!="correct") {Console.WriteLine("answer.txt must contain correct");Environment.Exit(7);}
            return;
        }
        if(args.Length==1 && args[0]=="check"){ if(File.Exists(Path.Combine(Dir,"fail-check"))){Console.WriteLine("fixture verification failed");Environment.Exit(9);} return;}
        if(args.Length==1 && args[0]=="sleeper"){Thread.Sleep(Timeout.Infinite);return;}
        string line;
        while((line=Console.ReadLine())!=null){
            var msg=Json.Deserialize<Dictionary<string,object>>(line);
            if(!msg.ContainsKey("method")){
                if(Mode("delivery-mode") && Convert.ToString(msg["id"])=="assignment") File.WriteAllText(Path.Combine(Dir,"lead-feedback"),line);
                if(Convert.ToString(msg["id"])=="assignment") Done("Worker batch returned.");
                if(Convert.ToString(msg["id"])=="report") Done("Finished assignment.");
                if(Convert.ToString(msg["id"])=="posted-image") Done("Here is the revised layout.");
                if(Convert.ToString(msg["id"])=="block") Done("Waiting for the release input.");
                continue;
            }
            string method=(string)msg["method"];
            if(method=="initialized") continue;
            var p=(Dictionary<string,object>)msg["params"];
            var id=msg["id"];
            if(method=="initialize") Send(new {id=id,result=new {}});
            if(method=="config/read") Send(new {id=id,result=new {config=new {model="fixture-fast",model_reasoning_effort="low"}}});
            if(method=="model/list"){
                if (Mode("catalog.hold")) {
                    File.WriteAllText(Path.Combine(Dir, "catalog.ready"), "ready");
                    while (Mode("catalog.hold")) Thread.Sleep(40);
                }
                var low=new {reasoningEffort="low"}; var high=new {reasoningEffort="high"};
                Send(new {id=id,result=new {data=new[]{new {model="fixture-fast",defaultReasoningEffort="low",supportedReasoningEfforts=new[]{low},isDefault=true},
                    new {model="fixture-deep",defaultReasoningEffort="high",supportedReasoningEfforts=new[]{low,high},isDefault=false}}}});
            }
            if(method=="thread/start" || method=="thread/resume"){
                if (Mode("check-isolation") && (Environment.GetEnvironmentVariable("CODEX_WINDOWS_REGISTERED_CORE") != null ||
                    !Object.Equals(p["sandbox"], "workspace-write") || !Object.Equals(p["approvalPolicy"], "never")))
                    throw new InvalidDataException("Unexpected provider process identity or sandbox settings");
                session=p.ContainsKey("threadId")?(string)p["threadId"]:ThreadId;
                worker=((string)p["developerInstructions"]).StartsWith("You are a Workshop worker");
                studio=p.ContainsKey("ephemeral") && Object.Equals(p["ephemeral"],true);
                string instructions = (string)p["developerInstructions"];
                studioKind = instructions.StartsWith("Summarize the supplied") ? "summary" :
                    instructions.StartsWith("Design a cohesive") ? "design" : "image";
                Send(new {id=id,result=new {thread=new {id=session}}});
            }
            if(method=="thread/loaded/list") Send(new {id=id,result=new {data=new[]{session}}});
            if(method=="turn/start"){
                if(File.Exists(Path.Combine(Dir,"record-inference")))
                    File.WriteAllText(Path.Combine(Dir,(studio ? "studio-"+studioKind+"-" : worker ? "worker-" : "lead-")+Guid.NewGuid()+".json"),Json.Serialize(p));
                turn=Guid.NewGuid().ToString();
                Send(new {id=id,result=new {turn=new {id=turn}}});
                var first=(Dictionary<string,object>)((System.Collections.IList)p["input"])[0];
                if (Delivery((string)first["text"])) continue;
                while (Mode("notice.hold")) Thread.Sleep(40);
                if(studio){
                    if(File.Exists(Path.Combine(Dir,"studio.hold"))) Thread.Sleep(Timeout.Infinite);
                    if (studioKind == "summary") {
                        if (File.Exists(Path.Combine(Dir,"summary.fail"))) {
                            Send(new {method="turn/completed",@params=new {threadId=session,turn=new {id=turn,status="failed",error=new {message="Summary fixture offline"}}}});
                        } else Done("The sample functions compute tile values.");
                        continue;
                    }
                    if (studioKind == "design") {
                        var part = new {name="Lantern",shape="box",size=new[]{1,1,1},center=new[]{0,0,0.5},rotation=new[]{0,0,0},color="3377aa"};
                        Done(Json.Serialize(new {parts=new[]{part}}));
                        continue;
                    }
                    Send(new {method="item/completed", @params=new {threadId=session,turnId=turn,
                        item=new {type="imageGeneration",status="completed",savedPath=Path.Combine(Dir,"studio-image.png")}}});
                    Done("Image generated.");
                    continue;
                }
                if(worker){Worker((string)first["text"]);continue;}
                string message=(string)first["text"];
                if (message == "Show the reference drawing" && Mode("chat-image.png")) {
                    Send(new {method="item/completed", @params=new {threadId=session,turnId=turn,
                        item=new {type="imageGeneration",status="completed",savedPath=Path.Combine(Dir,"chat-image.png")}}});
                    Done("Here is the reference."); continue;
                }
                if (message == "Move the highlighted area to the right" && Mode("chat-image.png")) {
                    File.Copy(Path.Combine(Dir,"capture-fixture.png"),"revised.png",true);
                    Send(new {id="posted-image",method="item/tool/call",@params=new {threadId=session,turnId=turn,
                        callId="posted-image",tool="workshop_send_image",arguments=new {path="revised.png",caption="Revised layout"}}});
                    continue;
                }
                if (Mode("small-talk")) { Done("Hello!"); continue; }
                if (Mode("formatted-reply")) {
                    Done("## Project structure\n\nThe app opens the **current folder** and remembers the last active chat.\n\n```rust\nfn workspace_title(name: &str) -> String {\n    // Keep the folder name in the title bar.\n    format!(\"Workshop / {}\", name)\n}\n```\n\nCall `workspace_title` when the selected project changes.");
                    continue;
                }
                if (Mode("blocker")) {
                    File.WriteAllText("answer.txt","partial");
                    Send(new {id="block",method="item/tool/call",@params=new {threadId=session,turnId=turn,callId="block",tool="workshop_blocked",arguments=new {reason="Missing release input"}}});
                    continue;
                }
                if (Mode("runner-error")) {
                    Send(new {method="item/completed",@params=new {threadId=session,turnId=turn,item=new {type="commandExecution",status="failed",exitCode=-1,aggregatedOutput="Unable to start the test command runner"}}});
                    Done("Work could not run"); continue;
                }
                if (Mode("direct-repair")) {
                    Send(new {method="item/started",@params=new {threadId=session,turnId=turn,item=new {type="fileChange"}}});
                    File.WriteAllText("answer.txt",message.Contains("answer.txt must contain correct") ? "correct" : "wrong");
                    Done("Updated the answer"); continue;
                }
                if (Mode("autonomous")) {
                    if (message.StartsWith("Workshop verification")) {
                        if (!message.Contains("right.txt contains rigth")) throw new InvalidDataException("Missing review finding in repair turn");
                        var fix = new {key="fix-right",title="Write fix-right",deliverable="Fix spelling",acceptance="right.txt contains right",files=new[]{"right.txt"},depends_on=new string[0]};
                        Delegate(new List<object>{fix, Audit("left","right","fix-right")});
                    } else {
                        Delegate(new List<object>{Assignment("left"),Assignment("right"),Audit("left","right"),Assignment("publish","audit")});
                    }
                    continue;
                }
                var tasks=new List<object>{Assignment("left"),Assignment("right")};
                if(File.Exists(Path.Combine(Dir,"mixed"))){
                    tasks.Add(Assignment("guide","left"));
                    tasks.Add(Assignment("summary","guide","right"));
                }
                Send(new {id="assignment",method="item/tool/call",@params=new {threadId=session,turnId=turn,callId="call",tool="workshop_delegate",arguments=new {tasks=tasks}}});
            }
            if(method=="turn/steer"){
                Send(new {id=id,result=new {}});
                var direction=(Dictionary<string,object>)((System.Collections.IList)p["input"])[0];
                Delivery((string)direction["text"]);
            }
            if(method=="turn/interrupt"){
                Send(new {id=id,result=new {}});
                Send(new {method="turn/completed",@params=new {threadId=session,turn=new {id=turn,status="interrupted"}}});
            }
        }
    }
}
