sed -i 's/use candle_core::{Tensor, Device, Result as CResult, Module};\nuse std::ops::Add;/use candle_core::{Tensor, Device, Result as CResult, Module};/' src/models.rs
