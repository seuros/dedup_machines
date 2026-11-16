# frozen_string_literal: true
require_relative 'lib/dedup_machines/version'

Gem::Specification.new do |spec|
  spec.name = 'dedup_machines'
  spec.version = DedupMachines::VERSION
  spec.authors = ['Abdelkader Boudih']
  spec.email = ['oss@seuros.com']
  spec.summary = 'Idempotency keys and duplicate-message claims for Ruby'
  spec.description = 'A bounded claim ledger: claim a key before acting on a message, and the next copy ' \
                     'learns it is in progress or already done instead of causing a second side effect. ' \
                     'Claim ids are checked on completion, records expire by TTL, and a full ledger ' \
                     'refuses new keys instead of evicting. Bound to Ruby from the no_std ' \
                     'dedup-machines crate: in memory, not durable storage or exactly-once delivery.'
  spec.homepage = 'https://github.com/seuros/dedup_machines'
  spec.license = 'MIT'
  spec.required_ruby_version = '>= 4.0.0'
  spec.metadata['source_code_uri'] = spec.homepage
  spec.metadata['rubygems_mfa_required'] = 'true'
  spec.metadata['cargo_crate_name'] = 'dedup_machines_native'
  spec.metadata['cargo_manifest_path'] = 'ext/dedup_machines_native/ffi/Cargo.toml'
  spec.files = Dir.glob(%w[
    lib/**/*.rb sig/**/*.rbs ext/**/*.{rs,rb,toml,md} Cargo.toml
    Cargo.lock README.md CHANGELOG.md LICENSE.txt
  ]).select { |p| File.file?(p) }
    .reject { |p| p.end_with?('/tests.rs') || p.include?('/tests/') }
  spec.require_paths = ['lib']
  spec.extensions = ['ext/dedup_machines_native/extconf.rb']
  spec.add_dependency 'rb_sys', '~> 0.9', '>= 0.9.130'
end
