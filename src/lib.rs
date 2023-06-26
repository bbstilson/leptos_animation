use instant::Instant;
use std::{
    collections::VecDeque,
    ops::{Add, Sub},
    time::Duration,
};

use leptos::{create_effect, create_memo, create_trigger, leptos_dom::helpers::AnimationFrameRequestHandle, on_cleanup, provide_context, request_animation_frame_with_handle, store_value, use_context, Scope, Signal, SignalGet, StoredValue, Trigger, Memo, SignalWith};

pub mod animation_target;
pub mod easing;
pub mod tween;

/// The AnimationContext handles updating all animated values and calling request_animation_frame().
/// It is required to provide one in a parent context before calling create_animated_signal()
#[derive(Copy, Clone)]
pub struct AnimationContext {
    pub ticks: Trigger,
    animation_frame_request_handle: StoredValue<Option<AnimationFrameRequestHandle>>,
}

impl AnimationContext {
    /// Sets up an AnimationContext for this scope and all child scopes
    pub fn provide(cx: Scope) -> AnimationContext {
        let ticks = create_trigger(cx);
        let animation_frame_request_handle =
            store_value(cx, Option::<AnimationFrameRequestHandle>::None);

        let animation_context = AnimationContext {
            ticks,
            animation_frame_request_handle,
        };
        provide_context(cx, animation_context.clone());

        on_cleanup(cx, move || {
            if let Some(handle) = animation_frame_request_handle.get_value() {
                handle.cancel()
            }
        });

        animation_context
    }

    /// Manually request a new animation frame. It is normally not necessary to call this
    pub fn request_animation_frame(&self) {
        // Prevent multiple animation frame requests from existing simultaneously
        if self.animation_frame_request_handle.get_value().is_none() {
            let this = self.clone();

            self.animation_frame_request_handle.set_value(Some(
                request_animation_frame_with_handle(move || {
                    this.animation_frame_request_handle.set_value(None);
                    this.ticks.notify();
                })
                    .unwrap(),
            ));
        }
    }
}

/// An AnimationTarget is a target value for the animation system to ease towards to. Additional properties
pub struct AnimationTarget<T> {
    pub target: T,

    /// The time for which the animation plays. Defaults to 0.5 seconds
    pub duration: Duration,

    /// The easing method to apply during the animation. Defaults to easeInCubic
    pub easing: Easing,

    /// The mode specifies how to deal with running animation. This can be used to add, overwrite or cancel running animations.
    /// The default mode is to start a new animation, see `AnimationMode` for more information
    pub mode: AnimationMode,
}

pub enum AnimationMode {
    /// Starts a new animation on top
    Start,
    StartOrReplace,
    SnapOrReplace,
    Snap,
}

pub type Easing = fn(f64) -> f64;

struct Animation<T, I> {
    from: T,
    to: T,
    to_i: I,
    start: Instant,
    duration: Duration,
    easing: Easing,
}

impl<T, I> Animation<T, I> {
    fn is_finished(&self) -> bool {
        Instant::now() > self.start + self.duration
    }

    fn progress(&self) -> f64 {
        (self.easing)((Instant::now() - self.start).as_secs_f64() / self.duration.as_secs_f64())
    }
}

enum AnimationStatus<T, I> {
    BeforeFirst,
    Static(T),
    /// The VecDeque is guarenteed to contain atleast one animation. All animations are guarenteed
    /// to be sorted in reverse order of when they started with the most recent one in front and the oldest one in the back
    /// This state does not automatically reset to NoAnimation after the animations are finished
    Running {
        to: T,
        to_i: I,
        animations: VecDeque<Animation<T, I>>,
    },
}

impl<T: Clone, I> AnimationStatus<T, I> {
    fn remove_finished_animations(&mut self) {
        if let AnimationStatus::Running { to, animations, .. } = self {
            animations.retain(|animation| !animation.is_finished());
            if animations.len() == 0 {
                *self = AnimationStatus::Static(to.clone());
            }
        }
    }
}

pub fn create_animated_signal<T, I>(
    cx: Scope,
    source: impl Fn() -> AnimationTarget<T> + 'static,
    tween: fn(&T, &T, f64) -> I,
) -> Signal<I>
    where
        T: 'static,
        T: Clone, //where V: Clone, I: PartialEq {
        I: Clone,
        I: Sub<I, Output=I>,
        I: Add<I, Output=I>,
{
    let context: AnimationContext = use_context(cx)
        .expect("No AnimationContext present, call AnimationContext::new() in a parent scope");
    let animation_status = store_value(cx, AnimationStatus::<T, I>::BeforeFirst);

    // TODO: update doc: Special in-between signal used to update the animation status that only runs based on source changes
    create_effect(cx, move |_| {
        let animation_target = source();

        animation_status.update_value(|animation_status| {
            animation_status.remove_finished_animations(); // Makes sure that there are no finished animations that mess with the below logic
            match animation_status {
                // The very first animation can not be played as there is no 'from' value. It results directly in the Static state.
                AnimationStatus::BeforeFirst => {
                    *animation_status = AnimationStatus::Static(animation_target.target)
                }

                // Starting an animation from a Static state
                AnimationStatus::Static(state) => match animation_target.mode {
                    AnimationMode::Start | AnimationMode::StartOrReplace => {
                        let to_i =
                            tween(&animation_target.target, &animation_target.target, 1.0);
                        *animation_status = AnimationStatus::Running {
                            to: animation_target.target.clone(),
                            to_i: to_i.clone(),
                            animations: VecDeque::from([Animation {
                                from: state.clone(),
                                to: animation_target.target,
                                to_i,
                                start: Instant::now(),
                                duration: animation_target.duration,
                                easing: animation_target.easing,
                            }]),
                        }
                    }
                    AnimationMode::SnapOrReplace | AnimationMode::Snap => {
                        *animation_status = AnimationStatus::Static(animation_target.target)
                    }
                },
                // Start an animation from a running state
                AnimationStatus::Running {
                    to,
                    to_i,
                    animations,
                } => match animation_target.mode {
                    AnimationMode::Start => {
                        let new_to_i =
                            tween(&animation_target.target, &animation_target.target, 1.0);

                        animations.push_front(Animation {
                            from: to.clone(),
                            to: animation_target.target.clone(),
                            to_i: new_to_i.clone(),
                            start: Instant::now(),
                            duration: animation_target.duration,
                            easing: animation_target.easing,
                        });
                        *to = animation_target.target;
                        *to_i = new_to_i;
                    }
                    // This arm can only be reached when there are still live animations, so we perform the 'replace' operation
                    AnimationMode::StartOrReplace | AnimationMode::SnapOrReplace => {
                        *to = animation_target.target.clone();
                        *to_i = tween(&animation_target.target, &animation_target.target, 1.0);
                        animations.front_mut().unwrap().to = animation_target.target;
                    }
                    AnimationMode::Snap => {
                        *animation_status = AnimationStatus::Static(animation_target.target)
                    }
                },
            }
        });
        context.request_animation_frame();
    });

    // This is a crude way to filter signals
    enum SignalUpdate {
        Ignore,
        Update,
    }
    impl PartialEq for SignalUpdate {
        fn eq(&self, other: &Self) -> bool {
            match other {
                SignalUpdate::Ignore => true,
                SignalUpdate::Update => false
            }
        }
    }

    /// Internal signal that fires on animation ticks while 
    let animation_tick = create_memo(cx, move |_| {
        context.ticks.track(); // 

        animation_status.update_value(|animation_status| {
            animation_status.remove_finished_animations();
        });
        animation_status.with_value(|animation_status| {
            match animation_status {
                AnimationStatus::BeforeFirst | AnimationStatus::Static(_) => SignalUpdate::Ignore,
                AnimationStatus::Running { .. } => SignalUpdate::Update,
            }
        })
    });

    Signal::derive(cx, move || {
        animation_tick.track();
        
        let i: I = animation_status.with_value(|animation_status| match animation_status {
            AnimationStatus::BeforeFirst => unreachable!(""),
            AnimationStatus::Static(state) => tween(state, state, 1.0),
            AnimationStatus::Running {
                animations, to_i, ..
            } => {
                // Keep this signal updated in the animation loop
                context.ticks.track();
                context.request_animation_frame();

                // Add all animation results to a single value
                animations.iter().fold(to_i.clone(), |acc, animation| {
                    let animation_value =
                        tween(&animation.from, &animation.to, animation.progress());

                    acc + (animation_value - animation.to_i.clone())
                })
            }
        });
        i
    })
}
