use instant::Instant;
use std::{
    collections::VecDeque,
    ops::{Add, Sub},
    time::Duration,
};

use leptos::{
    create_effect, create_signal, provide_context, request_animation_frame, store_value,
    use_context, ReadSignal, Scope, Signal, SignalGet, SignalSet, StoredValue, WriteSignal,
};

pub mod animation_target;
pub mod easing;
pub mod tween;

/// The AnimationTick is a placeholder value for use in a special signal that fires on each tick
#[derive(Clone)]
pub struct AnimationTick;

#[derive(Clone)]
struct AnimationFrameRequested(bool);

/// The AnimationContext handles updating all animated values and calling request_animation_frame().
/// It is required to provide one in a parent context before calling create_animated_signal()
#[derive(Clone)]
pub struct AnimationContext {
    pub ticks: ReadSignal<AnimationTick>,
    set_ticks: WriteSignal<AnimationTick>,
    animation_frame_requested: StoredValue<AnimationFrameRequested>,
}

impl AnimationContext {
    /// Sets up an AnimationContext for this scope and all child scopes
    pub fn provide(cx: Scope) -> AnimationContext {
        let (ticks, set_ticks) = create_signal(cx, AnimationTick);
        let animation_frame_requested = store_value(cx, AnimationFrameRequested(false));

        let animation_context = AnimationContext {
            ticks,
            set_ticks,
            animation_frame_requested,
        };
        provide_context(cx, animation_context.clone());

        animation_context
    }

    /// Manually request a new animation frame. It is normally not necessary to call this
    pub fn request_animation_frame(&self) {
        // Prevent multiple animation frame requests from existing simultaneously
        if !self.animation_frame_requested.get_value().0 {
            let animation_context = self.clone();
            request_animation_frame(move || {
                animation_context
                    .animation_frame_requested
                    .set_value(AnimationFrameRequested(false));
                animation_context.set_ticks.set(AnimationTick)
            });
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
    to_value: I,
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
    BeforeFirstAnimation,
    NoAnimation(T),
    /// The VecDeque is guarenteed to contain atleast one animation. All animations are guarenteed
    /// to be sorted in reverse order of when they started with the most recent one in front and the oldest one in the back
    /// This state does not automatically reset to NoAnimation after the animations are finished
    RunningAnimations(VecDeque<Animation<T, I>>),
}

impl<T: Clone, I> AnimationStatus<T, I> {
    fn remove_finished_animations(&mut self) {
        if let AnimationStatus::RunningAnimations(animations) = self {
            let to = animations.front().unwrap().to.clone();
            animations.retain(|animation| !animation.is_finished());
            if animations.len() == 0 {
                *self = AnimationStatus::NoAnimation(to);
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
    I: Sub<I, Output = I>,
    I: Add<I, Output = I>,
{
    let context: AnimationContext = use_context(cx)
        .expect("No AnimationContext present, call AnimationContext::new() in a parent scope");
    let animation_status = store_value(cx, AnimationStatus::<T, I>::BeforeFirstAnimation);

    // Special in-between signal used to update the running_animations that only runs based on source changes
    let animation_changed = Signal::derive(cx, move || {
        let animation_target = source();

        animation_status.update_value(|animation_status| {
            animation_status.remove_finished_animations(); // Makes sure that there are no finished animations that mess with the below logic
            match animation_status {
                // The very first animation can not be played as there is no 'from' value. It results directly in a NoAnimation state.
                AnimationStatus::BeforeFirstAnimation => {
                    *animation_status = AnimationStatus::NoAnimation(animation_target.target)
                }

                // Starting an animation from a NoAnimation state
                AnimationStatus::NoAnimation(previous) => match animation_target.mode {
                    AnimationMode::Start | AnimationMode::StartOrReplace => {
                        let to_value =
                            tween(&animation_target.target, &animation_target.target, 1.0);
                        *animation_status =
                            AnimationStatus::RunningAnimations(VecDeque::from([Animation {
                                from: previous.clone(),
                                to: animation_target.target,
                                to_value,
                                start: Instant::now(),
                                duration: animation_target.duration,
                                easing: animation_target.easing,
                            }]))
                    }
                    AnimationMode::SnapOrReplace | AnimationMode::Snap => {
                        *animation_status = AnimationStatus::NoAnimation(animation_target.target)
                    }
                },
                // Start an animation from a running state
                AnimationStatus::RunningAnimations(animations) => match animation_target.mode {
                    AnimationMode::Start => {
                        let to_value =
                            tween(&animation_target.target, &animation_target.target, 1.0);
                        animations.push_front(Animation {
                            from: animations.front().unwrap().to.clone(),
                            to: animation_target.target,
                            to_value,
                            start: Instant::now(),
                            duration: animation_target.duration,
                            easing: animation_target.easing,
                        })
                    }
                    // This arm can only be reached when there are still live animations, so we perform the 'replace' operation
                    AnimationMode::StartOrReplace | AnimationMode::SnapOrReplace => {
                        animations.front_mut().unwrap().to = animation_target.target
                    }
                    AnimationMode::Snap => {
                        *animation_status = AnimationStatus::NoAnimation(animation_target.target)
                    }
                },
            }
        });
        ()
    });

    Signal::derive(cx, move || {
        animation_changed.get();

        animation_status
            .update_value(|animation_status| animation_status.remove_finished_animations());
        let i: I = animation_status.with_value(|animation_status| match animation_status {
            AnimationStatus::BeforeFirstAnimation => unreachable!(""),
            AnimationStatus::NoAnimation(value) => tween(&value, &value, 1.0),
            AnimationStatus::RunningAnimations(animations) => {
                // Keep this signal updated in the animation loop
                context.ticks.get();
                context.request_animation_frame();

                // Add all animation results to a single value
                animations.iter().fold(
                    animations.front().unwrap().to_value.clone(),
                    |acc, animation| {
                        let animation_value =
                            tween(&animation.from, &animation.to, animation.progress());

                        acc + (animation_value - animation.to_value.clone())

                        // acc: 900
                        // animation_value: 400
                        // sub: -500
                        // add it to the accumulator

                        // Result: 900 + (-500) + ... + ... = 400

                        // animation.progress();
                        // return acc;
                    },
                )
            }
        });
        i
    })
}
