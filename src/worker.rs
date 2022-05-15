use crossbeam::channel::{Receiver, Sender};
use std::fmt::{Display, Formatter};
use std::thread::JoinHandle;

#[derive(Debug, Clone)]
pub enum WorkerError {
    /// It was impossible to send a Quit message to the worker thread
    WorkerSendError,
    /// The worker thread panicked when we attempted to join it
    WorkerPanic(String),
}

impl Display for WorkerError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl From<Box<dyn std::any::Any + Send>> for WorkerError {
    fn from(e: Box<dyn std::any::Any + Send>) -> Self {
        if let Some(s) = e.downcast_ref::<&str>() {
            WorkerError::WorkerPanic(s.to_string())
        } else {
            WorkerError::WorkerPanic("non-string panic".to_string())
        }
    }
}

impl std::error::Error for WorkerError {}

pub enum WorkerControlMessage {
    Quit,
}

pub struct Worker<ControlMessageType>
where
    ControlMessageType: From<WorkerControlMessage> + Send + 'static,
{
    control_sender: Sender<ControlMessageType>,
    worker_handle: JoinHandle<()>,
}

impl<ControlMessageType> Worker<ControlMessageType>
where
    ControlMessageType: From<WorkerControlMessage> + Send + 'static,
{
    pub fn new<MessageType, FnWorker>(message_sender: Sender<MessageType>, f: FnWorker) -> Self
    where
        MessageType: Send + 'static,
        FnWorker: FnOnce(Sender<MessageType>, Receiver<ControlMessageType>) -> () + Send + 'static,
    {
        Self::new_with_capacity(message_sender, f, 100)
    }

    pub fn new_with_capacity<MessageType, FnWorker>(
        message_sender: Sender<MessageType>,
        f: FnWorker,
        cap: usize,
    ) -> Self
    where
        MessageType: Send + 'static,
        FnWorker: FnOnce(Sender<MessageType>, Receiver<ControlMessageType>) -> () + Send + 'static,
    {
        let (control_sender, control_receiver) = crossbeam::channel::bounded(cap);
        let worker_handle = std::thread::spawn(move || {
            f(message_sender, control_receiver);
        });
        Self {
            control_sender,
            worker_handle,
        }
    }

    pub fn new_consumer<FnWorker>(f: FnWorker) -> Self
    where
        FnWorker: FnOnce(Receiver<ControlMessageType>) -> () + Send + 'static,
    {
        Self::new_consumer_with_capacity(f, 100)
    }

    pub fn new_consumer_with_capacity<FnWorker>(f: FnWorker, cap: usize) -> Self
    where
        FnWorker: FnOnce(Receiver<ControlMessageType>) -> () + Send + 'static,
    {
        let (control_sender, control_receiver) = crossbeam::channel::bounded(cap);
        let worker_handle = std::thread::spawn(move || {
            f(control_receiver);
        });
        Self {
            control_sender,
            worker_handle,
        }
    }

    pub fn stop(self) -> Result<(), WorkerError> {
        if let Err(_) = self
            .control_sender
            .send(ControlMessageType::from(WorkerControlMessage::Quit))
        {
            // The worker thread message receiver is dead so the worker must also be dead.
            // We should still join it to avoid leaving a detached thread.
            self.worker_handle.join()?;
            return Err(WorkerError::WorkerSendError);
        }
        Ok(self.worker_handle.join()?)
    }

    pub fn control_sender(&self) -> Sender<ControlMessageType> {
        self.control_sender.clone()
    }
}
