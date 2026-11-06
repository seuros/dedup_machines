# frozen_string_literal: true
require 'bundler/gem_tasks'
require 'rake/testtask'
require 'rb_sys/extensiontask'

RbSys::ExtensionTask.new('dedup_machines_native', Gem::Specification.load('dedup_machines.gemspec')) do |e|
  e.lib_dir = 'lib/dedup_machines_native'
  e.tmp_dir = 'tmp/rb_sys'
end
Rake::TestTask.new(:ruby_test) do |t|
  t.libs << 'lib'
  t.pattern = 'test/**/*_test.rb'
end
task test: %i[compile ruby_test]
task :rbs do
  sh 'rbs', '-I', 'sig', 'validate'
end
task default: %i[test rbs]

Dir[File.join(__dir__, 'tasks/*.rake')].sort.each { |f| load f }
