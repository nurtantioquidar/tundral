using System;
using System.Collections.Generic;
using Shop.Models;
using static System.Math;
using Repo = Shop.Services.IRepository<Shop.Models.User>;
using Clock = System.DateTime;

namespace Shop.Services
{
    [Obsolete("Use the new service")]
    public sealed class UserService : IUserService, IDisposable
    {
        private readonly Repo _repository;
        private readonly ILogger _logger;

        public UserService(Repo repository, ILogger logger)
        {
            _repository = repository;
            _logger = logger;
        }

        public User Register(string name)
        {
            var user = User.Of(name);
            user.AddAddress(new User.Address());
            _repository.Save(user);
            this.Notify(user);
            Audit(name.Shout());
            _logger.Log(Formatter.Format(user.GetName()));
            return user;
        }

        public User Register(User user, Point location)
        {
            _repository.Save(user);
            return user;
        }

        public IReadOnlyList<string> Names()
        {
            var names = new List<string>();
            foreach (var u in _repository.All())
            {
                names.Add(u.GetName().Slug());
            }
            return names;
        }

        public T Pick<T>(IEnumerable<T> items) where T : Entity
        {
            foreach (var item in items)
            {
                return item;
            }
            throw new InvalidOperationException();
        }

        public int Largest(int a, int b) => Max(a, b);

        private void Notify(User user)
        {
            Console.WriteLine(user.Describe());
        }

        private static void Audit(string message)
        {
            Clock now = Clock.UtcNow;
            Console.WriteLine($"{now}: {message}");
        }

        public void Dispose()
        {
        }
    }

    public interface IUserService
    {
        User Register(string name);
    }

    public interface ILogger
    {
        void Log(string message);
    }
}
