use std::{ops::Deref, borrow::Cow};

use axum::{response::IntoResponse, Json};
use dashmap::{DashMap, mapref::{multiple::RefMulti, one::Ref}};
use shared::{HasWaitId, MsgId, Msg, MsgSigned, MsgTaskRequest, MsgState, MsgTaskResult, MsgEmpty, MsgSocketRequest, MsgSocketResult};
use tokio::sync::broadcast;

pub trait Task {
    type Result: Msg + Clone;

    fn get_results(&self) -> Cow<'_, Vec<Self::Result>>;
    fn push_result(&mut self, result: Self::Result);
}

// impl<State: MsgState> Task for MsgTaskRequest<State> {
//     type Result = MsgSigned<MsgTaskResult<State>>;

//     fn get_results(&self) -> &Vec<Self::Result> {
//         &self.results
//     }

//     fn push_result(&mut self, result: Self::Result) {
//         self.results.push(result)
//     }
// }

impl<State: MsgState> Task for MsgSocketRequest<State> {
    type Result = MsgSigned<MsgSocketResult>;

    fn get_results(&self) -> Cow<'_, Vec<Self::Result>> {
        if let Some(result) = &self.result {
            // Find a better way
            Cow::Owned(vec![result.clone()])
        } else {
            Cow::Owned(Vec::with_capacity(0))
        }
    }

    fn push_result(&mut self, result: Self::Result) {
        self.result = Some(result);
    }
}

pub struct TaskManager<T: HasWaitId<MsgId> + Task + Msg> {
    tasks: DashMap<MsgId, MsgSigned<T>>,
    new_tasks: broadcast::Sender<MsgId>,
    /// Send the index at which the new result for the given Task was inserted
    new_results: DashMap<MsgId, broadcast::Sender<usize>>
}

impl<T: HasWaitId<MsgId> + Task + Msg> TaskManager<T> {
    pub fn subscribe_results(&self, task_id: &MsgId) -> broadcast::Receiver<usize> {
        self.new_results
            .get(&task_id)
            .expect("An index sender is registered every time a task gets added")
            .subscribe()
    }

    pub fn subscribe_tasks(&self) -> broadcast::Receiver<MsgId> {
        self.new_tasks.subscribe()
    }

    pub fn get_tasks_by(&self, filter: impl Fn(&T) -> bool) -> impl Iterator<Item = impl Deref<Target = MsgSigned<T>> + '_> {
        self.tasks
            .iter()
            .filter(move |entry| filter(&entry.msg))
    }

    pub fn get(&self, task_id: &MsgId) -> Option<impl Deref<Target = MsgSigned<T>> + '_> {
        self.tasks.get(task_id)
    }

    /// This will push the result to the given task by its id
    /// Returns true if the given task exists flase otherwise
    pub fn put_result(&self, task_id: &MsgId, result: T::Result) -> bool {
        let Some(mut entry) = self.tasks.get_mut(task_id) else {
            return false;
        };
        let n = entry.msg.get_results().len();
        entry.msg.push_result(result);
        // We dont care if noone is listening
        _ = self.new_results
            .get(task_id)
            .expect("This task id must be present because it is present at the start of the function")
            .send(n);
        true
    }

    pub fn post_task(&self, task: MsgSigned<T>) {
        let id = task.wait_id();
        self.tasks.insert(task.wait_id(), task);
        // We dont care if noone is listening
        _ = self.new_tasks.send(id);
    }
}

// I think this would be a great idea
// struct Task<T> {
//     data: T
// }
