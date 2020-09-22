/*
* Copyright 2018-2020 TON DEV SOLUTIONS LTD.
*
* Licensed under the SOFTWARE EVALUATION License (the "License"); you may not use
* this file except in compliance with the License.
*
* Unless required by applicable law or agreed to in writing, software
* distributed under the License is distributed on an "AS IS" BASIS,
* WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
* See the License for the specific TON DEV software governing permissions and
* limitations under the License.
*/

use sha2::{Digest};

pub fn sha256(bytes: &Vec<u8>) -> Vec<u8> {
    let mut hasher = sha2::Sha256::new();
    hasher.input(bytes);
    hasher.result().to_vec()
}

pub fn sha512(bytes: &Vec<u8>) -> Vec<u8> {
    let mut hasher = sha2::Sha512::new();
    hasher.input(bytes);
    hasher.result().to_vec()
}