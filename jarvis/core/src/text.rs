use super::*;
impl Engine {
    pub async fn text_turn(self:Arc<Self>,text:String,speak:bool){
        let text=text.trim();if text.is_empty(){return;}
        let phrase = self.listener.phrase();
        let command = if phrase == crate::wake_profiles::DEFAULT_PHRASE { text.to_owned() }
            else { fastcmd::command_for_phrase(text, &phrase) };
        let command = command.as_str();
        if fastcmd::is_stop(command){
            let e=self.clone();let all=!self.talking() && fastcmd::is_stop_all(command);
            let _=tokio::task::spawn_blocking(move||if all{e.stop("написали стоп")}else{e.end_chat("написали стоп")}).await;
            self.journal.note("стоп",text,"");return;
        }
        let s=self.store.get();let p=s.active_profile();
        let llm_engine = p.effective_llm_engine();
        let llm_model = p.effective_llm_model();
        let model = if !llm_model.is_empty() { llm_model } else if !llm_engine.is_empty() { llm_engine } else { &p.backend_model };
        let id=self.journal.new_turn("текст",model);
        self.journal.update(id,|t|{t.text=text.into();t.live=true;});
        let epoch=self.speech_epoch.load(Ordering::SeqCst);
        let work=async {
            if let Some(question)=fastcmd::clarification(command){return Ok(question.into());}
            if let Some(answer)=self.host.command(command).await{return Ok(answer);}
            if let Some(answer)=self.music.control(&self.host,command).await{return Ok(answer);}
            if let Some(answer)=self.tools.ha_vacuum(command).await{return Ok(answer);}
            if let Some(answer)=self.tools.smartlife_fast(command).await{return Ok(answer);}
            if let Some(command)=fastcmd::alice_skill_for(command,s.use_home("yandexmusic")){
                let answer=format!("Передал Алисе: {command}");tokio::task::spawn_blocking(move||alice::text(&command,false)).await.map_err(|e|e.to_string())??;return Ok(answer);
            }
            if let Some(answer)=self.tools.fast(command).await{return Ok(answer);}
            let (context,defs)=self.tools.prepare().await;
            let instructions=format!("{}{}{}\n{context}",self.instructions(&s,&p),rules::TEXT_RULES,rules::BACKEND_RULES);
            crate::llm::ask(&self.host,&s,&p,&instructions,text,&self.tools,defs).await
        };
        let result=tokio::select!{
            value=tokio::time::timeout(Duration::from_secs(90),work)=>value.unwrap_or_else(|_|Err("Запрос не завершился за 90 с".into())),
            _=async{
                loop {
                    let wait = self.speech_notify.notified();
                    if self.speech_epoch.load(Ordering::SeqCst) != epoch { break; }
                    wait.await;
                }
            }=>Err("Запрос отменён".into()),
        };
        self.journal.update(id,|t|{t.live=false;match &result{Ok(answer)=>t.answer=answer.clone(),Err(error)=>t.error=error.clone()}});
        self.journal.remember(id);
        if speak && self.speech_epoch.load(Ordering::SeqCst)==epoch {
            if let Ok(answer)=result {let _=tokio::task::spawn_blocking(move||self.say_text(&answer)).await;}
        }
    }
}
