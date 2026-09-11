/// Kotlin style scope functions for better flow
pub trait ScopeFns {
    /// Transforms `f` into fluent style
    fn then<T>(self, f: impl FnOnce(Self) -> T) -> T
    where
        Self: Sized;
    /// Runs `f` with a shared reference to self. Useful for debug statements
    fn also(self, f: impl FnOnce(&Self)) -> Self;
    /// Runs `f` with a mutable reference to self. Good for initializers
    fn run(self, f: impl FnOnce(&mut Self)) -> Self;
}

impl<T> ScopeFns for T {
    fn then<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }

    fn also(self, f: impl FnOnce(&Self)) -> Self {
        f(&self);
        self
    }

    fn run(mut self, f: impl FnOnce(&mut Self)) -> Self {
        f(&mut self);
        self
    }
}
