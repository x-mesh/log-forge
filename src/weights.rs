use rand::distributions::WeightedIndex;
use std::io;

pub(crate) fn checked<I>(weights: I) -> io::Result<WeightedIndex<f64>>
where
    I: IntoIterator<Item = f64>,
    I::IntoIter: Clone,
{
    let values = weights.into_iter();
    let mut sum = 0.0;
    for weight in values.clone() {
        if !weight.is_finite() || weight < 0.0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "weights must be finite and nonnegative",
            ));
        }
        sum += weight;
        if !sum.is_finite() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "weight total must be finite",
            ));
        }
    }
    WeightedIndex::new(values).map_err(io::Error::other)
}
