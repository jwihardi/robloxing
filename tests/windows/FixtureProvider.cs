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
        FinishCodexReply(text);
    }
    static void FinishCodexReply(string text) {
        Send(new { method="item/completed", @params=new {threadId=session,turnId=turn,item=new {type="agentMessage",id="reply",text=text}} });
        Send(new { method="thread/tokenUsage/updated", @params=new {threadId=session,tokenUsage=new {total=new {totalTokens=100}}} });
        Send(new { method="turn/completed", @params=new {threadId=session,turn=new {id=turn,status="completed"}} });
    }
    static object Assignment(string key, params string[] dependencies) {
        if (key == "left" && Mode("level-design"))
            return new {key=key, specialty="level_design", title="Write left", deliverable="Block out the map", acceptance="File contains its name", files=new[]{"left.txt"}, depends_on=dependencies};
        return new {key=key,title="Write "+key,deliverable="Write "+key+".txt",acceptance="File contains its name",files=new[]{File.Exists(Path.Combine(Dir,"overlap"))?".":key+".txt"},depends_on=dependencies};
    }
    static bool Mode(string name) { return File.Exists(Path.Combine(Dir, name)); }
    static void RecordQueuePrompt(string frame) {
        if (Mode("record-queue")) File.AppendAllText(Path.Combine(Dir,"queue-wire.jsonl"),frame+"\n");
    }
    static void WaitForInit() {
        if (!Mode("init.hold")) return;
        File.WriteAllText(Path.Combine(Dir,"init.ready"), "ready");
        while (Mode("init.hold")) Thread.Sleep(20);
    }
    static string GoalEvidence(bool first) {
        if (first) {
            File.WriteAllText("first.txt","one");
            return "Created the first fixture file.";
        }
        File.WriteAllText("second.txt","two");
        if (File.ReadAllText("first.txt") != "one") throw new InvalidDataException("First file changed");
        return "Verified both fixture files.";
    }
    static string GoalReply() {
        bool first = !File.Exists("first.txt");
        string evidence = GoalEvidence(first);
        return first ? evidence : "[Goal complete] " + evidence;
    }
    static object CatalogPlugin(string id, string label, string detail) {
        bool installed = File.Exists(Path.Combine(Dir,"sample.plugin")) && File.ReadAllText(Path.Combine(Dir,"sample.plugin")) == id;
        return new {id=id,name=id.Split('@')[0],installed=installed,enabled=installed,installPolicy="AVAILABLE",availability="AVAILABLE",
            @interface=new {displayName=label,shortDescription=detail}};
    }
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
                if (passed) Report("complete", "Joined outputs pass acceptance");
                else Send(new {id="report",method="item/tool/call",@params=new {threadId=session,turnId=turn,callId="report",tool="workshop_report_result",
                    arguments=new {status="needs_changes",reason="right.txt contains rigth; expected right",summary="Read both joined files and compared their contents with their names."}}});
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
        int config = Array.IndexOf(args,"--mcp-config");
        if(Mode("capture-mcp") && config >= 0)
            File.WriteAllText(Path.Combine(Dir,"mcp-claude.json"),args[config+1]);
        int instructions = Array.IndexOf(args,"--append-system-prompt");
        worker = instructions >= 0 && args[instructions+1].StartsWith("You are a Workshop worker");
        int resume = Array.IndexOf(args,"--resume");
        session = resume >= 0 ? args[resume+1] : ThreadId;
        string line;
        while ((line=Console.ReadLine())!=null) {
            var frame = Json.Deserialize<Dictionary<string,object>>(line);
            switch ((string)frame["type"]) {
                case "control_request":
                    if (Convert.ToString(((Dictionary<string,object>)frame["request"])["subtype"]) == "initialize") WaitForInit();
                    if (Convert.ToString(frame["request_id"]) == "workshop-usage") {
                        Send(new {type="control_response",response=new {subtype="success",request_id=frame["request_id"],response=new {
                            rate_limits_available=true,rate_limits=new {five_hour=new {utilization=25},seven_day=new {utilization=48}}}}});
                        break;
                    }
                    if (Convert.ToString(frame["request_id"]) == "workshop-steer" && Mode("delivery-mode")) {
                        Send(new {type="result",subtype="error_during_execution",is_error=true,terminal_reason="aborted_by_interrupt"});
                        break;
                    }
                    var models = new object[] {
                        new {value="default", displayName="Default (recommended)", resolvedModel="claude-sonnet-5-5", supportedEffortLevels=new[]{"low","high"}},
                        new {value="sonnet", displayName="Sonnet", resolvedModel="claude-sonnet-5-5", supportedEffortLevels=new[]{"low","high"}},
                        new {value="haiku", displayName="Haiku", resolvedModel="claude-haiku-4-5-20251001"}
                    };
                    Send(new {type="control_response",response=new {subtype="success",request_id=frame["request_id"],response=new {models=models}}});
                    break;
                case "user":
                    RecordQueuePrompt(line);
                    Send(new {type="system",subtype="init",session_id=session});
                    var msg=(Dictionary<string,object>)frame["message"];
                    var blocks=(System.Collections.ArrayList)msg["content"];
                    var first=(Dictionary<string,object>)blocks[0];
                    if (((string)first["text"]).Contains("Usage fixture greeting")) {
                        Send(new {type="stream_event",@event=new {type="content_block_start",content_block=new {type="thinking"}}});
                        while(Mode("thinking.hold")) Thread.Sleep(30);
                        File.AppendAllText(Path.Combine(Dir, "claude-greetings"), Process.GetCurrentProcess().Id+"\n");
                        Done("Hello."); break;
                    }
                    if (Delivery((string)first["text"])) break;
                    if (worker) { Worker((string)first["text"]); break; }
                    if (Mode("goal-mode")) { Done(GoalReply()); break; }
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
    static void Acp(string provider) {
        string line; string sid=ThreadId;
        while ((line=Console.ReadLine())!=null) {
            var frame=Json.Deserialize<Dictionary<string,object>>(line);
            var id=frame["id"]; var p=(Dictionary<string,object>)frame["params"];
            switch(Convert.ToString(frame["method"])) {
                case "initialize": WaitForInit(); Send(new {id=id,result=new {protocolVersion=1}}); break;
                case "session/list":
                    if (Mode("provider-setup") && !File.Exists(Path.Combine(Dir, "antigravity.session"))) {
                        Send(new {id=id,error=new {code=-32000,message="Authentication required"}});
                    } else {
                        Send(new {id=id,result=new {sessions=new object[0]}});
                    }
                    break;
                case "session/new": case "session/load":
                    if(Mode("capture-mcp") && line.Contains("workshop"))
                        File.WriteAllText(Path.Combine(Dir,"mcp-"+provider+".json"),Json.Serialize(p["mcpServers"]));
                    if(p.ContainsKey("sessionId")) sid=(string)p["sessionId"];
                    Send(new {id=id,result=new {sessionId=sid,modes=new {currentModeId="accept-edits"},
                        models=new {currentModelId="gemini-test-high",availableModels=new[]{new {modelId="gemini-test-high",name="Gemini Test (High)"}}}}}); break;
                case "session/set_model": case "session/set_mode": Send(new {id=id,result=new {}});break;
                case "session/prompt":
                    RecordQueuePrompt(line);
                    Send(new {method="session/update",@params=new {sessionId=sid,update=new {sessionUpdate="agent_thought_chunk",content=new {type="text",text="fixture"}}}});
                    while(Mode("thinking.hold")) Thread.Sleep(30);
                    string reply = Mode("goal-mode") ? GoalReply() : "Hello.";
                    Send(new {method="session/update",@params=new {sessionId=sid,update=new {sessionUpdate="agent_message_chunk",content=new {type="text",text=reply}}}});
                    Send(new {id=id,result=new {stopReason="end_turn"}});break;
            }
        }
    }
    static void Main(string[] args) {
        if(args.Length==4 && args[0]=="mcp" && args[1]=="get" && args[2]=="blender") {
            Send(new {enabled=true,transport=new {type="stdio",command=Environment.GetCommandLineArgs()[0],args=new[]{"--fixture-blender"}}});
            return;
        }
        if(args.Length==1 && args[0]=="--fixture-blender") {
            string input;
            while((input=Console.ReadLine())!=null) {
                var frame=Json.Deserialize<Dictionary<string,object>>(input);
                if(!frame.ContainsKey("id")) continue;
                var id=frame["id"];
                if((string)frame["method"]=="initialize") {
                    Send(new {jsonrpc="2.0",id=id,result=new {protocolVersion="2025-06-18",capabilities=new {tools=new {}},serverInfo=new {name="fixture",version="1"}}});
                } else if((string)frame["method"]=="tools/list") {
                    var tool=new {name="get_objects_summary",inputSchema=new {type="object"}};
                    Send(new {jsonrpc="2.0",id=id,result=new {tools=new[]{tool}}});
                }
            }
            return;
        }
        if(Array.IndexOf(args,"--acp")>=0 || (args.Length>=2 && args[0]=="acp" && Array.IndexOf(args,"--stdio")>=0)){Acp(args[0]=="acp" ? "antigravity" : "gemini");return;}
        if (Mode("provider-setup") && !Mode("antigravity.session") &&
            (args.Length == 1 && args[0] == "models" || args.Length == 4 && args[0] == "-p" && (args[1] == "/model" || args[1] == "/usage"))) {
            File.WriteAllText(Path.Combine(Dir, "unexpected-auth-probe"), "1");
        }
        if (args.Length == 1 && args[0] == "models") {
            Console.WriteLine("gemini-test-high    Gemini Test (High)");
            Console.WriteLine("gemini-test-low     Gemini Test (Low)");
            return;
        }
        if (args.Length == 4 && args[0] == "-p" && args[1] == "/usage") {
            Send(new { command=new {data=new {groups=new[] {new {buckets=new[] {
                new {id="gemini-5h",window="5h",remaining_fraction=0.67},
                new {id="gemini-weekly",window="weekly",remaining_fraction=0.42}
            }}}}} }); return;
        }
        if (args.Length == 2 && args[0] == "acp" && args[1] == "--login") {
            File.AppendAllText(Path.Combine(Dir, "agy-login-count"), "1");
            Console.Error.WriteLine("Open this link in a browser to log in with Google:");
            Console.Error.WriteLine("https://accounts.google.com/o/oauth2/auth?state=fixture");
            if (Console.ReadLine() != "fixture-code") { Environment.Exit(1); return; }
            File.WriteAllText(Path.Combine(Dir, "antigravity.session"), "fixture"); return;
        }
        if (args.Length == 4 && args[0] == "-p" && args[1] == "/model") {
            if (Mode("provider-setup") && !File.Exists(Path.Combine(Dir, "antigravity.session"))) return;
            Send(new { command=new {data=new {id="gemini-test-high"}} }); return;
        }
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
            if(args[1]=="list"){
                if(Mode("recommend.refresh.hold") && File.Exists(file)){
                    File.WriteAllText(Path.Combine(Dir,"recommend.refresh.ready"),"ready");
                    while(Mode("recommend.refresh.hold")) Thread.Sleep(30);
                }
                Send(new {installed=File.Exists(file) ? new object[]{File.ReadAllText(file)==sculpt.pluginId ? (object)sculpt : plugin} : new object[0],available=new object[]{plugin,official}});return;
            }
            if(args[1]=="add" || args[1]=="install"){
                while(Mode("plugin.install.hold")) Thread.Sleep(30);
                File.WriteAllText(file,args[2]);return;
            }
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
                if(Convert.ToString(msg["id"])=="assignment") {
                    if (Mode("autonomous") && !Object.Equals(((Dictionary<string,object>)msg["result"])["success"],true)
                        && !line.Contains("right.txt contains rigth")) throw new InvalidDataException("Missing worker reason in dispatch result");
                    Done("Worker batch returned.");
                }
                if(Convert.ToString(msg["id"])=="report") Done("Finished assignment.");
                if(Convert.ToString(msg["id"])=="posted-image") Done("Here is the revised layout.");
                if(Convert.ToString(msg["id"])=="pack-inspect") {
                    var result=(Dictionary<string,object>)msg["result"];
                    if(!Object.Equals(result["success"],true)) throw new InvalidDataException("Could not inspect the pack");
                    var picture=(Dictionary<string,object>)((System.Collections.IList)result["contentItems"])[1];
                    var url=Convert.ToString(picture["imageUrl"]);
                    if(!url.StartsWith("data:image/png;base64,")) throw new InvalidDataException("Missing native image block");
                    File.WriteAllBytes(Path.Combine(Dir,"inspected-member.png"),Convert.FromBase64String(url.Substring(url.IndexOf(',')+1)));
                    Send(new {id="chat-asset",method="item/tool/call",@params=new {threadId=session,turnId=turn,callId="chat-asset",tool="workshop_generate_asset",
                        arguments=new {kind="model",brief="A matching dock lantern",style_pack="harbor",style_asset="crate"}}});
                }
                if(Convert.ToString(msg["id"])=="chat-asset") {
                    var result=(Dictionary<string,object>)msg["result"];
                    File.AppendAllText(Path.Combine(Dir,"chat-asset-results.jsonl"),Json.Serialize(result)+"\n");
                    Done(Object.Equals(result["success"],true) ? "Your asset is ready." : "The asset could not be created.");
                }
                if(Convert.ToString(msg["id"])=="goal-result") {
                    var result=(Dictionary<string,object>)msg["result"];
                    if(!Object.Equals(result["success"],true)) throw new InvalidDataException("Workshop rejected the goal report");
                    Done("Checked the fixture files.");
                }
                if(Convert.ToString(msg["id"])=="block") Done("Waiting for the release input.");
                continue;
            }
            string method=(string)msg["method"];
            if(method=="initialized") continue;
            var p=(Dictionary<string,object>)msg["params"];
            var id=msg["id"];
            if(method=="initialize") { WaitForInit(); Send(new {id=id,result=new {}}); }
            if(method=="plugin/list"){
                File.AppendAllText(Path.Combine(Dir,"browse.requests"),"list\n");
                if(Mode("browse.fail")) { Send(new {id=id,error=new {code=-32000,message="Fixture marketplace offline"}}); continue; }
                var official = new List<object>{CatalogPlugin("calendar@openai-curated-remote","Calendar","Plan events")};
                if(Mode("many.plugins")){
                    for(int n=1;n<=65;n++) official.Add(CatalogPlugin("extra-"+n+"@openai-curated-remote","Extra "+n.ToString("D2"),"Large catalog fixture"));
                }
                var markets = new object[] {
                    new {name="fixture",plugins=new[]{CatalogPlugin("sample@fixture","Sample plugin","Community plugin")}},
                    new {name="openai-curated-remote",plugins=official}
                };
                Send(new {id=id,result=new {marketplaces=markets}}); continue;
            }
            if(method=="config/read") Send(new {id=id,result=new {config=new {model="fixture-fast",model_reasoning_effort="low"}}});
            if(method=="account/rateLimits/read") Send(new {id=id,result=new {rateLimits=new {limitId="codex",
                primary=new {windowDurationMins=300,usedPercent=12},secondary=new {windowDurationMins=10080,usedPercent=69}}}});
            if(method=="model/list"){
                if (Mode("catalog.hold")) {
                    File.WriteAllText(Path.Combine(Dir, "catalog.ready"), "ready");
                    while (Mode("catalog.hold")) Thread.Sleep(40);
                }
                var low=new {reasoningEffort="low"}; var high=new {reasoningEffort="high"};
                var models=new List<object> {new {model="fixture-fast",defaultReasoningEffort="low",supportedReasoningEfforts=new[]{low},isDefault=true},
                    new {model="fixture-deep",defaultReasoningEffort="high",supportedReasoningEfforts=new[]{low,high},isDefault=false}};
                if (Mode("large-task-catalog")) {
                    for (int i=1;i<=28;i++) models.Add(new {model="fixture-option-"+i.ToString("00"),defaultReasoningEffort="low",supportedReasoningEfforts=new[]{low},isDefault=false});
                }
                Send(new {id=id,result=new {data=models}});
            }
            if(method=="thread/start" || method=="thread/resume"){
                if(Mode("capture-mcp")) File.WriteAllText(Path.Combine(Dir,"mcp-codex.json"),Json.Serialize(p["config"]));
                if (Mode("check-isolation") && (Environment.GetEnvironmentVariable("CODEX_WINDOWS_REGISTERED_CORE") != null ||
                    !Object.Equals(p["sandbox"], "danger-full-access") || !Object.Equals(p["approvalPolicy"], "never")))
                    throw new InvalidDataException("Unexpected provider process identity or sandbox settings");
                session=p.ContainsKey("threadId")?(string)p["threadId"]:ThreadId;
                worker=((string)p["developerInstructions"]).StartsWith("You are a Workshop worker");
                studio=p.ContainsKey("ephemeral") && Object.Equals(p["ephemeral"],true);
                string instructions = (string)p["developerInstructions"];
                studioKind = instructions.StartsWith("Summarize the supplied") ? "summary" :
                    instructions.StartsWith("Design a game sound effect") ? "sound" : instructions.StartsWith("Design a cohesive") ? "design" : "image";
                if (worker && Mode("delivery-mode")) Send(new {id=id,result=new {thread=new {id=session},model="fixture-deep",reasoningEffort="high"}});
                else Send(new {id=id,result=new {thread=new {id=session}}});
            }
            if(method=="thread/loaded/list") Send(new {id=id,result=new {data=new[]{session}}});
            if(method=="turn/start"){
                RecordQueuePrompt(line);
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
                    if (studioKind == "sound") {
                        Done(Json.Serialize(new {layers=new[]{new {wave="sine",start=0,duration=0.3,from_hz=550,to_hz=900,gain=0.8,attack=0.01,release=0.2}}}));
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
                if(Mode("pack-handoff") && message.StartsWith("Build a dock.")) {
                    Send(new {id="pack-inspect",method="item/tool/call",@params=new {threadId=session,turnId=turn,callId="pack-inspect",tool="workshop_asset_pack",
                        arguments=new {action="inspect",pack="harbor",asset="crate"}}});
                    continue;
                }
                if (Mode("chat-assets")) {
                    object spec=null;
                    if(message.StartsWith("Make a pickup sound")) spec=new {kind="sound",title="Coin pickup",brief="Two bright chimes"};
                    if(message.StartsWith("Draw a forest icon")) spec=new {kind="image",brief="Forest icon",model="fixture-deep",effort="high"};
                    if(message.StartsWith("Draw the moonlit harbor")) spec=new {kind="image",brief="Moonlit harbor"};
                    if(message.StartsWith("Refine the marked icon")) spec=new {kind="image",brief="Refined icon",reference_image="chat"};
                    if(message.StartsWith("Build a lantern")) spec=new {kind="model",title="Copper lantern",brief="Design a lantern with a copper frame."};
                    if(spec!=null) {
                        Send(new {id="chat-asset",method="item/tool/call",@params=new {threadId=session,turnId=turn,
                            callId="chat-asset",tool="workshop_generate_asset",arguments=spec}});
                        continue;
                    }
                }
                if (Mode("goal-mode")) {
                    if (Mode("goal.hold")) continue;
                    bool firstFile=!File.Exists("first.txt");
                    string evidence=GoalEvidence(firstFile);
                    Send(new {id="goal-result",method="item/tool/call",@params=new {threadId=session,turnId=turn,
                        callId="goal-result",tool="workshop_goal_result",arguments=new {status=firstFile ? "progress" : "complete",evidence=evidence}}});
                    continue;
                }
                if (message == "Show the reference drawing" && Mode("chat-image.png")) {
                    Send(new {method="item/started", @params=new {threadId=session,turnId=turn,item=new {type="imageGeneration"}}});
                    while(Mode("image.hold")) Thread.Sleep(30);
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
                if (Mode("linked-files")) {
                    var note = "delivery-note.txt";
                    if (File.Exists(note)) {
                        File.WriteAllText(note,"revised");
                        Done("Updated the same file.");
                    } else {
                        File.WriteAllText(note,"first");
                        File.WriteAllBytes("Lobby.png",Convert.FromBase64String("iVBORw0KGgoAAAANSUhEUgAAAAwAAAAICAIAAABChommAAAAEUlEQVR4nGNoe3CAIGIYVQQAL2HOQZG1B4YAAAAASUVORK5CYII="));
                        var zip = new byte[22]; zip[0]=80; zip[1]=75; zip[2]=5; zip[3]=6;
                        File.WriteAllBytes("Game.zip",zip);
                        Done("**Ready.** [Download the project](Game.zip)\n\n![Lobby preview](Lobby.png)");
                    }
                    continue;
                }
                if (Mode("scroll.hold")) {
                    Send(new {method="item/started",@params=new {threadId=session,turnId=turn,item=new {type="mcpToolCall",tool="js"}}});
                    while (Mode("tool.hold")) Thread.Sleep(30);
                    var reply = new System.Text.StringBuilder();
                    for (int i=0; i<16; i++) reply.AppendFormat("[Part {0}](part-{0}.zip)\n\n", i);
                    Send(new {method="item/agentMessage/delta",@params=new {threadId=session,turnId=turn,itemId="reply",delta=reply.ToString()}});
                    while (Mode("scroll.hold")) Thread.Sleep(30);
                    FinishCodexReply(reply.ToString() + "## Delivery\n\n[Final file](finished.zip)");
                    continue;
                }
                if (Mode("chat-media")) {
                    if (message.Contains("Show the clip again")) File.WriteAllText(Path.Combine(Dir, "media-followup"), "yes");
                    var photo = new Uri(Path.Combine(Dir, "photo.png")).AbsoluteUri;
                    var clip = new Uri(Path.Combine(Dir, "clip.webm")).AbsoluteUri;
                    Done("[Photo](" + photo + ")\n\n[Playtest](" + clip + ")");
                    continue;
                }
                if (Mode("formatted-reply")) {
                    Done("## Project structure\n\nThe app opens the **current folder** and remembers the last active chat.\n\n```rust\nfn workspace_title(name: &str) -> String {\n    // Keep the folder name in the title bar.\n    format!(\"Workshop / {}\", name)\n}\n```\n\nCall `workspace_title` when the selected project changes.");
                    continue;
                }
                if (Mode("blocker")) {
                    File.WriteAllText("answer.txt","partial");
                    var reason = File.ReadAllText(Path.Combine(Dir, "blocker")).Trim();
                    Send(new {id="block",method="item/tool/call",@params=new {threadId=session,turnId=turn,callId="block",tool="workshop_blocked",arguments=new {reason=reason.Length > 0 ? reason : "Missing release input"}}});
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
                        while (Mode("repair.hold")) Thread.Sleep(30);
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
                if(Mode("late-receipt")) {
                    Done("The original turn has finished.");
                    Send(new {id=id,error=new {code=-32000,message="Previous turn ended"}});
                    continue;
                }
                if(Mode("reject-direction")) { Send(new {id=id,error=new {code=-32000,message="Turn no longer accepts directions"}}); continue; }
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
